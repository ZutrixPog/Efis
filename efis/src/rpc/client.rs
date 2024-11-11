use std::time::Duration;

use tokio::io::{
    AsyncWriteExt,
    AsyncReadExt,
};
use tokio::sync::Mutex;
use tokio::net::TcpStream;

use super::Serialize;

const DEFAULT_TIMEOUT_SECS: u64 = 50;

// TODO: add auth
pub struct Client {
    conn: Mutex<TcpStream>, }

impl Client {
    pub async fn connect(addr: String) -> Self {
        let conn = TcpStream::connect(addr).await.unwrap();

        Client {
            conn: Mutex::new(conn),
        }
    }

    pub async fn call(&self, method: String, req: &dyn Serialize) -> anyhow::Result<String>  {
        let req = format!("{} {}", method, req.serialize()).into_bytes();

        let mut conn = self.conn.lock().await; 
        conn.write_all(&req).await?;

        let mut buff = Vec::with_capacity(1024);
        let _ = tokio::time::timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS), conn.read(&mut buff)).await?;
        println!("read res {:?}", buff.clone());

        Ok(String::from_utf8(buff)?)
    }
}

#[cfg(test)]
mod tests {
    use crate::rpc::client::Client;
    use crate::rpc::server::RpcServer;
    use crate::rpc::{Serialize, Deserialize, SerDe};
    use std::sync::Arc;
    use macros::{rpc_func, SerDe};

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Req {
        a: i32,
    }

    #[rpc_func]
    async fn rpc_test_fn(req: Req) -> anyhow::Result<Req> {
        Ok(Req { a: req.a * 2 })
    }

    #[tokio::test]
    async fn test_simple_req() {
        tokio::spawn(async {
            run_server().await; 
        });

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        let client = Client::connect("localhost:8080".to_owned()).await;
        let input = 12;
        let res = client.call("test".to_owned(), &Req{ a: input }).await;
        assert!(res.is_ok());
        let res = res.unwrap();
        println!("res: {}", res);

        let output = Req::deserialize(res.as_str()).unwrap();
        assert_eq!(output.a, input * 2);
    }

    async fn run_server() {
        let server = RpcServer::new();

        server.register_fn("test".to_owned(), Arc::new(rpc_test_fn)).await;

        let _ = server.run("localhost:8080").await;
    }
}
