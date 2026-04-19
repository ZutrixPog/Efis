use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;

use crate::rpc::{Deserialize, ErrorRes};

use super::Serialize;

enum ClientMsg {
    Call {
        payload: Vec<u8>,
        resp_tx: oneshot::Sender<anyhow::Result<String>>,
    },
    CallStream {
        payload: Vec<u8>,
        stream_tx: mpsc::Sender<String>,
    },
}

#[derive(Clone)]
pub struct Client {
    sender: mpsc::Sender<ClientMsg>,
    connected: Arc<AtomicBool>,
}

impl Client {
    pub fn new(addr: String) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let connected = Arc::new(AtomicBool::new(false));

        tokio::spawn(io_loop(addr, rx, connected.clone()));

        Self {
            sender: tx,
            connected,
        }
    }

    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub async fn call<T: Deserialize + Send>(
        &self,
        method: &str,
        req: &dyn Serialize,
    ) -> anyhow::Result<T> {
        let (resp_tx, resp_rx) = oneshot::channel();

        let payload = format!("{} {}\n", method, req.serialize()).into_bytes();

        self.sender
            .send(ClientMsg::Call { payload, resp_tx })
            .await
            .map_err(|_| anyhow::anyhow!("Client actor closed"))?;

        let raw_resp = resp_rx.await??;

        if let Ok(err) = ErrorRes::deserialize(&raw_resp) {
            return Err(anyhow::anyhow!(err.error));
        }
        T::deserialize(&raw_resp).map_err(|e| anyhow::anyhow!("DeDe Error: {}", e))
    }

    pub async fn call_stream<T: Deserialize + Send + 'static>(
        &self,
        method: &str,
        req: &dyn Serialize,
    ) -> anyhow::Result<mpsc::Receiver<T>> {
        let (stream_tx, mut stream_rx) = mpsc::channel(32);
        let (out_tx, out_rx) = mpsc::channel(32);

        let payload = format!("{} {}\n", method, req.serialize()).into_bytes();

        self.sender
            .send(ClientMsg::CallStream { payload, stream_tx })
            .await
            .map_err(|_| anyhow::anyhow!("Client actor closed"))?;

        tokio::spawn(async move {
            while let Some(raw) = stream_rx.recv().await {
                if let Ok(item) = T::deserialize(&raw) {
                    if out_tx.send(item).await.is_err() {
                        break;
                    }
                }
            }
        });

        Ok(out_rx)
    }
}

async fn io_loop(
    addr: String,
    mut inbox: mpsc::Receiver<ClientMsg>,
    connected_flag: Arc<AtomicBool>,
) {
    let mut stream: Option<TcpStream> = None;
    let mut buf = vec![0u8; 4096];

    loop {
        if stream.is_none() {
            match TcpStream::connect(&addr).await {
                Ok(s) => {
                    stream = Some(s);
                    connected_flag.store(true, Ordering::Relaxed);
                }
                Err(_) => {
                    connected_flag.store(false, Ordering::Relaxed);
                    sleep(Duration::from_millis(200)).await;
                    continue;
                }
            }
        }

        match inbox.recv().await {
            Some(msg) => {
                let s = stream.as_mut().unwrap();
                match msg {
                    ClientMsg::Call { payload, resp_tx } => {
                        if let Ok(r) = process_unary(s, &payload, &mut buf).await {
                            let _ = resp_tx.send(Ok(r));
                        } else {
                            stream = None;
                            let _ = resp_tx.send(Err(anyhow::anyhow!("Connection lost")));
                        }
                    }
                    ClientMsg::CallStream { payload, stream_tx } => {
                        if let Err(_) = process_stream(s, &payload, stream_tx, &mut buf).await {
                            stream = None;
                        }
                    }
                }
            }
            None => break,
        }
    }
}

async fn process_unary(
    stream: &mut TcpStream,
    payload: &[u8],
    buf: &mut Vec<u8>,
) -> anyhow::Result<String> {
    stream.write_all(payload).await?;
    stream.flush().await?;

    let mut accumulated = Vec::new();
    loop {
        let n = stream.read(buf).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("EOF"));
        }

        accumulated.extend_from_slice(&buf[..n]);
        if let Some(pos) = accumulated.iter().position(|&b| b == b'\n') {
            let line = accumulated.drain(..pos).collect::<Vec<_>>();
            return Ok(String::from_utf8(line)?);
        }
    }
}

async fn process_stream(
    stream: &mut TcpStream,
    payload: &[u8],
    tx: mpsc::Sender<String>,
    buf: &mut Vec<u8>,
) -> anyhow::Result<()> {
    stream.write_all(payload).await?;
    stream.flush().await?;

    let mut accumulated = Vec::new();
    loop {
        let n = stream.read(buf).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("EOF"));
        }

        accumulated.extend_from_slice(&buf[..n]);

        while let Some(pos) = accumulated.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = accumulated.drain(..=pos).collect();
            let line_str = String::from_utf8(line_bytes[..line_bytes.len() - 1].to_vec())?;

            if line_str == "done" {
                return Ok(());
            }

            if tx.send(line_str).await.is_err() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::rpc::client::Client;
    use crate::rpc::server::RpcServer;
    use crate::rpc::{Deserialize, RpcError, Serialize};
    use macros::{rpc_func, rpc_stream, SerDe};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Req {
        a: i32,
    }

    #[rpc_func]
    async fn rpc_test_fn(req: Req) -> Result<Req, RpcError> {
        Ok(Req { a: req.a * 2 })
    }

    #[rpc_func]
    async fn rpc_test_err_fn(req: Req) -> Result<Req, RpcError> {
        Err(RpcError::Internal(anyhow::anyhow!("error")))
    }

    #[rpc_stream]
    async fn rpc_test_stream_fn(req: Req) -> Result<mpsc::Receiver<Req>, RpcError> {
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

        let client = Client::new("localhost:8080".to_owned());
        let input = 12;
        let res = client.call("test", &Req { a: input }).await;
        assert!(res.is_ok());
        let res: Req = res.unwrap();

        assert_eq!(res.a, input * 2);

        let res = client.call::<Req>("test_err", &Req { a: input }).await;
        assert!(res.is_err());

        let mut res = client
            .call_stream::<Req>("test_stream", &Req { a: input })
            .await;
        assert!(res.is_ok());
        let mut vals = Vec::new();
        while let Some(msg) = res.as_mut().unwrap().recv().await {
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
