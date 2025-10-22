use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};

use crate::rpc::{Deserialize, ErrorRes};

use super::Serialize;

const DEFAULT_TIMEOUT_SECS: u64 = 50;
const BUFF_SIZE: usize = 512;

// TODO: add auth
pub struct Client {
    addr: String,
    conn: Mutex<TcpStream>,
}

impl Client {
    pub async fn connect(addr: String) -> Self {
        let conn = TcpStream::connect(addr.clone()).await.unwrap();

        Client {
            addr: addr,
            conn: Mutex::new(conn),
        }
    }

    pub async fn call<T: Deserialize>(
        &self,
        method: String,
        req: &dyn Serialize,
    ) -> anyhow::Result<T> {
        let req = format!("{} {}\n", method, req.serialize()).into_bytes();

        let mut conn = self.conn.lock().await;
        conn.write_all(&req).await?;
        conn.flush().await?;

        let mut buff = vec![0u8; BUFF_SIZE];
        let n = tokio::time::timeout(
            Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            conn.read(&mut buff),
        )
        .await??;
        buff = buff[..n].to_vec();

        let res_str = String::from_utf8(buff)?;

        if let Ok(err_res) = ErrorRes::deserialize(&res_str) {
            return Err(anyhow::anyhow!(err_res.error));
        }

        T::deserialize(&res_str).map_err(|e| anyhow::anyhow!(e))
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
            _ = conn.write_all(&req).await;
            _ = conn.flush().await;

            let mut buff = vec![0u8; BUFF_SIZE];
            while let Ok(n) = conn.read(&mut buff).await {
                buff = buff[..n].to_vec();

                let res = String::from_utf8(buff.clone());
                if res.is_err() {
                    return;
                }
                let res_str = res.unwrap();

                let msgs = res_str.split("\n");
                for msg in msgs {
                    if msg.starts_with("done") {
                        drop(tx);
                        return;
                    }

                    let msg = T::deserialize(&msg);
                    if let Ok(msg) = msg {
                        _ = tx.send(msg).await;
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
