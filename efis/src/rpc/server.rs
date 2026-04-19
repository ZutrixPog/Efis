use super::RpcError;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::io::BufWriter;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, RwLock, Semaphore};
use tokio::time::{self, Duration};
use tokio_util::codec::{FramedRead, LinesCodec};
use tracing::{error, info, instrument, warn};

use crate::rpc::dispatcher::{Dispatcher, MiddlewareFunc, RpcFunc, RpcStreamFunc};
use crate::rpc::{ErrorRes, RpcStruct, Serialize};

const MAX_CONNECTIONS: usize = 1000;
pub const BUFF_SIZE: usize = 512;

pub struct RpcServer {
    dispatcher: Arc<RwLock<Dispatcher>>,
}

struct Listener {
    listener: TcpListener,
    limit_connections: Arc<Semaphore>,
    notify_shutdown: broadcast::Sender<()>,
    dispatcher: Arc<RwLock<Dispatcher>>,
    shutdown_complete_tx: mpsc::Sender<()>,
}

struct Handler {
    socket: TcpStream,
    dispatcher: Arc<RwLock<Dispatcher>>,
    shutdown: Shutdown,
    _shutdown_complete: mpsc::Sender<()>,
}

impl RpcServer {
    pub fn new() -> Self {
        Self {
            dispatcher: Dispatcher::new(),
        }
    }

    pub async fn run(&self, host: &str) -> anyhow::Result<()> {
        let listener = TcpListener::bind(host).await?;
        let (notify_shutdown, _) = broadcast::channel(1);
        let (shutdown_complete_tx, mut shutdown_complete_rx) = mpsc::channel(1);

        let mut server = Listener {
            listener,
            limit_connections: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            notify_shutdown,
            dispatcher: Arc::clone(&self.dispatcher),
            shutdown_complete_tx,
        };

        tokio::select! {
            res = server.run() => {
                if let Err(err) = res {
                    error!(cause = %err, "failed to accept");
                }
            }
        }

        let Listener {
            shutdown_complete_tx,
            notify_shutdown,
            ..
        } = server;

        drop(notify_shutdown);
        drop(shutdown_complete_tx);

        let _ = shutdown_complete_rx.recv().await;
        Ok(())
    }

    pub async fn register_fn(&self, method: String, rpc_fn: Arc<RpcFunc>) {
        self.dispatcher.write().await.register_fn(method, rpc_fn)
    }

    pub async fn register_stream_fn(&self, method: String, stream_fn: Arc<RpcStreamFunc>) {
        self.dispatcher
            .write()
            .await
            .register_stream_fn(method, stream_fn)
    }

    pub async fn register_struct(&self, st: &'static dyn RpcStruct) {
        self.dispatcher.write().await.register_struct(st);
    }

    pub async fn middleware(&self, middle: Arc<MiddlewareFunc>) {
        self.dispatcher.write().await.middleware(middle);
    }

    pub async fn register_input_stream(&self, mut rx: broadcast::Receiver<String>) {
        let dispatcher = self.dispatcher.clone();

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        let buf = msg.as_bytes();

                        let reader = dispatcher.read().await;
                        if let Err(err) = reader.dispatch_rpc(buf).await {
                            error!("failed to dispatch internal message: {}", err);
                        }
                    }

                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("lagged by {} messages", n);
                        continue;
                    }

                    Err(broadcast::error::RecvError::Closed) => {
                        warn!("sender closed; input task exiting");
                        break;
                    }
                }
            }
        });
    }
}

impl Listener {
    async fn run(&mut self) -> anyhow::Result<()> {
        info!("accepting connections");

        loop {
            let permit = self
                .limit_connections
                .clone()
                .acquire_owned()
                .await
                .unwrap();

            let socket = self.accept().await?;

            let handler = Handler {
                socket,
                dispatcher: Arc::clone(&self.dispatcher),
                shutdown: Shutdown::new(self.notify_shutdown.subscribe()),
                _shutdown_complete: self.shutdown_complete_tx.clone(),
            };

            tokio::spawn(async move {
                if let Err(err) = handler.run().await {
                    warn!(cause = ?err, "Disconnected");
                }
                drop(permit);
            });
        }
    }

    async fn accept(&mut self) -> anyhow::Result<TcpStream> {
        let mut backoff = 1;

        loop {
            match self.listener.accept().await {
                Ok((socket, _)) => return Ok(socket),
                Err(err) => {
                    if backoff > 64 {
                        return Err(err.into());
                    }
                }
            }

            time::sleep(Duration::from_secs(backoff)).await;
            backoff *= 2;
        }
    }
}

use futures::StreamExt;

impl Handler {
    #[instrument(skip(self))]
    async fn run(mut self: Self) -> anyhow::Result<()> {
        let (read_half, write_half) = tokio::io::split(self.socket);
        let mut reader = FramedRead::new(read_half, LinesCodec::new());
        let mut writer = BufWriter::new(write_half);

        loop {
            tokio::select! {
                _ = self.shutdown.recv() => {
                    return Ok(());
                }

                result = reader.next() => {
                    match result {
                        Some(Ok(line)) => {
                            let req = line.as_bytes().to_vec();

                            match self.dispatcher.read().await.dispatch_rpc(&req).await {
                                Ok(res) => {
                                    writer.write_all(&res).await?;
                                    writer.write_all(b"\n").await?;
                                    writer.flush().await?;
                                    continue;
                                },
                                Err(RpcError::MethodNotFound(_)) => {
                                },
                                Err(err) => {
                                    let err_msg = ErrorRes {
                                        error: err.to_string(),
                                    }
                                    .serialize();

                                    writer.write_all(err_msg.as_bytes()).await?;
                                    writer.write_all(b"\n").await?;
                                    writer.flush().await?;
                                    continue;
                                },
                            };

                            match self.dispatcher.read().await.dispatch_stream(&req).await {
                                Ok(mut stream) => {
                                    while let Some(msg) = stream.recv().await {
                                        if msg.contains("exit") {
                                            stream.close();
                                            break;
                                        }
                                        writer.write_all(msg.as_bytes()).await?;
                                        writer.write_all(b"\n").await?;
                                        writer.flush().await?;
                                    }
                                }
                                Err(err) => {
                                    let err_msg = ErrorRes {
                                        error: err.to_string(),
                                    }
                                    .serialize();

                                    writer.write_all(err_msg.as_bytes()).await?;
                                    writer.write_all(b"\n").await?;
                                    writer.flush().await?;
                                }
                            }
                        }
                        Some(Err(e)) => {
                            return Err(e.into());
                        }
                        None => {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}
#[derive(Debug)]
pub(crate) struct Shutdown {
    is_shutdown: bool,
    notify: broadcast::Receiver<()>,
}

impl Shutdown {
    pub(crate) fn new(notify: broadcast::Receiver<()>) -> Shutdown {
        Shutdown {
            is_shutdown: false,
            notify,
        }
    }

    // pub(crate) fn is_shutdown(&self) -> bool {
    //     self.is_shutdown
    // }

    pub(crate) async fn recv(&mut self) {
        if self.is_shutdown {
            return;
        }

        let _ = self.notify.recv().await;
        self.is_shutdown = true;
    }
}
