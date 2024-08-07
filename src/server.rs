use std::future::Future;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Semaphore};
use tokio::time::{self, Duration};
use tracing::{error, info, instrument, warn};

use crate::errors::ServiceError;
use crate::pubsub::PubSubGuard;
use crate::efis::Efis;
use crate::store::DatastoreGuard;

struct Listener {
    datastore_holder: DatastoreGuard,
    pubsub_holder: PubSubGuard,
    listener: TcpListener,
    limit_connections: Arc<Semaphore>,
    notify_shutdown: broadcast::Sender<()>,
    shutdown_complete_tx: mpsc::Sender<()>,
}

#[derive(Debug)]
struct Handler {
    svc: Efis,
    socket: TcpStream,
    shutdown: Shutdown,
    _shutdown_complete: mpsc::Sender<()>,
}

const MAX_CONNECTIONS: usize = 300;

pub async fn run(listener: TcpListener, shutdown: impl Future, backup_dur: Option<Duration>, persist_path: Option<String>) {
    let (notify_shutdown, _) = broadcast::channel(1);
    let (shutdown_complete_tx, mut shutdown_complete_rx) = mpsc::channel(1);
    let store = DatastoreGuard::new(backup_dur, persist_path).await;
    let pubsub = PubSubGuard::new();

    let mut server = Listener {
        listener,
        datastore_holder: store,
        pubsub_holder: pubsub,
        limit_connections: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        notify_shutdown,
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

            let store = self.datastore_holder.store();
            let pubsub = self.pubsub_holder.ps();

            let mut handler = Handler {
                svc: Efis::new(store, pubsub),
                socket: socket,
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
    async fn run(&mut self) -> Result<(), ServiceError> {

        while !self.shutdown.is_shutdown() {
            let mut buf = vec![0; 1024];
            let n = tokio::select! {
                n = self.socket.read(&mut buf) => n.unwrap_or(0),
                _ = self.shutdown.recv() => {
                    return Ok(());
                }
            };

            if n == 0 {
                return Err(ServiceError::InvalidValueType);
            }

            let command_str = std::str::from_utf8(&buf).unwrap_or("");
            let (tx, mut rx) = mpsc::channel(1024);
            let res = self.svc.process_cmd(command_str, tx).await;

            let mut res = if let Err(err) = res {
                if command_str.contains("FIN") {
                    return Ok(());
                } else if err == ServiceError::UnknownCommand {
                    error!("Command was executed with Error {}", err.to_string());
                } else {
                    error!("Command {} was executed with Error {}", command_str, err.to_string());
                }
                format!("ERR {}", err)
            } else {
                info!("Command {} was executed without Error", command_str);
                let response = res.unwrap();
                if response == "PS".to_string() {
                    while let Some(msg) = rx.recv().await {
                        let _ = self.socket.write(msg.as_bytes()).await;
                    }
                    continue;
                }

                response
            };
            res.push_str("\n");
            let _ = self.socket.write((res).as_bytes()).await;
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