use std::time::Duration;

use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tracing::error;

use crate::rpc::{Deserialize, ErrorRes};

use super::Serialize;

const DEFAULT_TIMEOUT_SECS: u64 = 50;
const BUFF_SIZE: usize = 512;

// TODO: add auth
pub struct Client {
    addr: String,
    conn: Arc<RwLock<TcpStream>>,
    trigger: mpsc::Sender<()>,
    connected: Arc<AtomicPtr<bool>>,
}

impl Client {
    pub async fn connect(addr: String) -> Self {
        let (tx, mut rx) = mpsc::channel(8);
        let mut conn = TcpStream::connect(addr.clone()).await;
        while conn.is_err() {
            error!("failed to connect to peer");
            conn = TcpStream::connect(addr.clone()).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let conn = Arc::new(RwLock::new(conn.unwrap()));

        let addr_clone = addr.clone();
        let conn_clone = conn.clone();
        let connected = &mut true;
        let connected_ptr = Arc::new(AtomicPtr::new(connected));

        let connected_ptr_clone = connected_ptr.clone();
        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(100);
            let max = Duration::from_secs(5);

            loop {
                if rx.recv().await.is_none() {
                    return;
                }

                unsafe {
                    *connected_ptr_clone.load(Ordering::Relaxed) = false;
                }
                loop {
                    match TcpStream::connect(&addr_clone).await {
                        Ok(s) => {
                            *conn_clone.write().await = s;
                            backoff = Duration::from_millis(100);
                            unsafe {
                                *connected_ptr_clone.load(Ordering::Relaxed) = true;
                            }
                            break;
                        }
                        Err(_) => {
                            let sleep = tokio::time::sleep(backoff);
                            tokio::select! {
                                _ = sleep => {},
                                Some(_) = rx.recv() => {},
                            }

                            backoff = (backoff * 2).min(max);
                        }
                    }
                }
            }
        });

        Self {
            addr,
            conn,
            trigger: tx,
            connected: connected_ptr,
        }
    }

    pub fn connected(&self) -> bool {
        unsafe { *self.connected.as_ref().load(Ordering::Relaxed) }
    }

    pub async fn call<T: Deserialize>(
        &self,
        method: String,
        req: &dyn Serialize,
    ) -> anyhow::Result<T> {
        let packet = format!("{} {}\n", method, req.serialize()).into_bytes();

        for _ in 0..1 {
            let mut conn = self.conn.as_ref().write().await;

            if conn.write_all(&packet).await.is_err() {
                let _ = self.trigger.send(()).await;
                continue;
            }

            if conn.flush().await.is_err() {
                let _ = self.trigger.send(()).await;
                continue;
            }

            let mut buff = vec![0u8; BUFF_SIZE];
            let res = tokio::time::timeout(
                Duration::from_secs(DEFAULT_TIMEOUT_SECS),
                conn.read(&mut buff),
            )
            .await;

            let n = match res {
                Ok(Ok(n)) => n,
                _ => {
                    let _ = self.trigger.send(()).await;
                    continue;
                }
            };

            let res_str = String::from_utf8_lossy(&buff[..n]).to_string();

            if let Ok(err) = ErrorRes::deserialize(&res_str) {
                return Err(anyhow::anyhow!(err.error));
            }

            return T::deserialize(&res_str).map_err(anyhow::Error::msg);
        }

        anyhow::bail!("failed to connect after retries")
    }

    pub async fn call_stream<T: Deserialize + Send + Sync + 'static>(
        &self,
        method: String,
        req: &dyn Serialize,
    ) -> anyhow::Result<mpsc::Receiver<T>> {
        let req = format!("{} {}\n", method, req.serialize()).into_bytes();

        let (tx, rx) = mpsc::channel(10);

        let mut conn = TcpStream::connect(self.addr.clone()).await?;

        tokio::spawn(async move {
            let _ = conn.write_all(&req).await;
            let _ = conn.flush().await;

            let mut buff = vec![0u8; BUFF_SIZE];

            loop {
                let n = match conn.read(&mut buff).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };

                let part = &buff[..n];
                let Ok(s) = String::from_utf8(part.to_vec()) else {
                    break;
                };

                for msg in s.split("\n") {
                    if msg.starts_with("done") {
                        drop(tx);
                        return;
                    }
                    if let Ok(v) = T::deserialize(&msg) {
                        let _ = tx.send(v).await;
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use crate::rpc::client::Client;
    use crate::rpc::server::RpcServer;
    use crate::rpc::{Deserialize, Serialize};
    use macros::{rpc_func, rpc_stream, SerDe};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Req {
        a: i32,
    }

    #[rpc_func]
    async fn rpc_test_fn(req: Req) -> anyhow::Result<Req> {
        Ok(Req { a: req.a * 2 })
    }

    #[rpc_func]
    async fn rpc_test_err_fn(req: Req) -> anyhow::Result<Req> {
        Err(anyhow::anyhow!("error"))
    }

    #[rpc_stream]
    async fn rpc_test_stream_fn(req: Req) -> anyhow::Result<mpsc::Receiver<Req>> {
        let (tx, rx) = mpsc::channel(10);
        tokio::spawn(async move {
            for i in 0..req.a {
                let res = Req { a: i };
                let _ = tx.send(res).await;
            }
            drop(tx);
        });
        Ok(rx)
    }

    #[tokio::test]
    async fn test_simple_req() {
        tokio::spawn(async {
            run_server().await;
        });

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        let client = Client::connect("localhost:8080".to_owned()).await;
        let input = 12;
        let res = client.call("test".to_owned(), &Req { a: input }).await;
        assert!(res.is_ok());
        let res: Req = res.unwrap();

        assert_eq!(res.a, input * 2);

        let res = client
            .call::<Req>("test_err".to_owned(), &Req { a: input })
            .await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().to_string(), "error".to_string());

        let mut res = client
            .call_stream::<Req>("test_stream".to_owned(), &Req { a: input })
            .await;
        assert!(res.is_ok());
        let mut vals = Vec::new();
        while let Some(msg) = res.as_mut().unwrap().recv().await {
            println!("{:?}", msg);
            vals.push(msg);
        }
        assert_eq!(input as usize, vals.len());
    }

    async fn run_server() {
        let server = RpcServer::new();

        server
            .register_fn("test".to_owned(), Arc::new(rpc_test_fn))
            .await;
        server
            .register_fn("test_err".to_owned(), Arc::new(rpc_test_err_fn))
            .await;
        server
            .register_stream_fn("test_stream".to_owned(), Arc::new(rpc_test_stream_fn))
            .await;

        let _ = server.run("localhost:8080").await;
    }
}
