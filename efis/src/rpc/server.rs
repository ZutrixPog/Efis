use nom::AsBytes;
use std::future::Future;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, RwLock, Semaphore};
use tokio::time::{self, Duration};
use tracing::{error, info, instrument, warn};

use crate::errors::RpcError;
use crate::rpc::dispatcher::Dispatcher;

const MAX_CONNECTIONS: usize = 1000;

pub struct RpcServer {
    dispatcher: Arc<Dispatcher>,
}

struct Listener {
    listener: TcpListener,
    limit_connections: Arc<Semaphore>,
    notify_shutdown: broadcast::Sender<()>,
    dispatcher: Arc<Dispatcher>,
    shutdown_complete_tx: mpsc::Sender<()>,
}

struct Handler {
    socket: TcpStream,
    dispatcher: Arc<Dispatcher>,
    shutdown: Shutdown,
    _shutdown_complete: mpsc::Sender<()>,
}

impl RpcServer {
    pub fn new() -> Self {
        Self {
            dispatcher: Arc::new(Dispatcher::new()),
        }
    }

    pub async fn run(&self, host: &str, shutdown: impl Future) -> anyhow::Result<()> {
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
            _ = shutdown => {
                info!("shutting down");
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

            let mut handler = Handler {
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

impl Handler {
    #[instrument(skip(self))]
    async fn run(&mut self) -> Result<(), RpcError> {
        while !self.shutdown.is_shutdown() {
            let mut buf = vec![0; 1024];
            let n = tokio::select! {
                n = self.socket.read(&mut buf) => n.unwrap_or(0),
                _ = self.shutdown.recv() => {
                    return Ok(());
                }
            };

            if n == 0 {
                return Err(RpcError::EmptyRequest);
            }

            //let (tx, mut rx) = mpsc::channel(1024);

            let res = self.dispatcher.as_ref().dispatch(&buf).await;
            if let Ok(r) = res {
                let _ = self.socket.write(&r).await;
            } else {
                let _ = self
                    .socket
                    .write(res.unwrap_err().to_string().as_bytes())
                    .await;
            }
        }

        Ok(())
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

    pub(crate) fn is_shutdown(&self) -> bool {
        self.is_shutdown
    }

    pub(crate) async fn recv(&mut self) {
        if self.is_shutdown {
            return;
        }

        let _ = self.notify.recv().await;
        self.is_shutdown = true;
    }
}
