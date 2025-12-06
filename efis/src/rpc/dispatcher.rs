use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use super::RpcStruct;

pub type RpcFunc = dyn Fn(String) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>>
    + Send
    + Sync
    + 'static;

pub type RpcStreamFunc = dyn Fn(String) -> Pin<Box<dyn Future<Output = anyhow::Result<mpsc::Receiver<String>>> + Send>>
    + Send
    + Sync
    + 'static;

pub type MiddlewareFunc = dyn Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;

pub struct Dispatcher {
    rpcs: HashMap<String, Arc<RpcFunc>>,
    streams: HashMap<String, Arc<RpcStreamFunc>>,
    middlewares: Vec<Arc<MiddlewareFunc>>,
}

impl Dispatcher {
    pub fn new() -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(Self {
            rpcs: HashMap::new(),
            streams: HashMap::new(),
            middlewares: Vec::new(),
        }))
    }

    pub fn register_fn(&mut self, method: String, rpc_fn: Arc<RpcFunc>) {
        self.rpcs.insert(method, rpc_fn);
    }

    pub fn register_stream_fn(&mut self, method: String, stream_fn: Arc<RpcStreamFunc>) {
        self.streams.insert(method, stream_fn);
    }

    pub fn register_struct(&mut self, st: &'static dyn RpcStruct) {
        st.register_fns(self);
    }

    pub fn middleware(&mut self, middle: Arc<MiddlewareFunc>) {
        self.middlewares.push(middle);
    }

    pub async fn dispatch_rpc(&self, req: &[u8]) -> anyhow::Result<Vec<u8>> {
        let req_str = String::from_utf8_lossy(req);
        let mut parts = req_str.split(" ").collect::<Vec<&str>>();
        let method = parts.remove(0).trim();

        for middleware in self.middlewares.iter() {
            middleware(String::from_utf8(req.to_vec()).unwrap()).await;
        }

        let rpc_fn = self
            .rpcs
            .get(method)
            .ok_or_else(|| anyhow::anyhow!("Method not found"))?;

        let response = rpc_fn(parts.join(" ")).await? + "\n";

        Ok(response.into_bytes())
    }

    pub async fn dispatch_stream(&self, req: &[u8]) -> anyhow::Result<mpsc::Receiver<String>> {
        let req_str = String::from_utf8_lossy(req);
        let mut parts = req_str.split(" ").collect::<Vec<&str>>();
        let method = parts.remove(0).trim();

        for middleware in self.middlewares.iter() {
            middleware(String::from_utf8(req.to_vec()).unwrap()).await;
        }

        let stream_fn = self
            .streams
            .get(method)
            .ok_or_else(|| anyhow::anyhow!("Method not found"))?;

        let response = stream_fn(parts.join(" ")).await?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::{Deserialize, Serialize};
    use macros::{rpc_func, rpc_impl, rpc_stream, rpc_struct, SerDe};
    use std::mem::MaybeUninit;
    use std::sync::Once;
    use std::time::Duration;

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Req {
        a: i32,
        b: f64,
        c: String,
        d: Vec<i32>,
    }

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Res {
        a: i32,
    }

    #[rpc_struct]
    struct Test {}

    #[rpc_impl]
    impl Test {
        pub fn singleton(t: Self) -> &'static Self {
            static mut SINGLETON: MaybeUninit<Test> = MaybeUninit::uninit();
            static ONCE: Once = Once::new();

            unsafe {
                ONCE.call_once(|| {
                    let singleton = t;
                    SINGLETON.write(singleton);
                });

                SINGLETON.assume_init_ref()
            }
        }

        #[rpc_func]
        pub async fn rpc_fn(&'static self, req: Req) -> anyhow::Result<Res> {
            Ok(Res { a: 12 })
        }

        #[rpc_stream]
        pub async fn rpc_stream_fn(&'static self, req: Req) -> anyhow::Result<mpsc::Receiver<Res>> {
            let (tx, rx) = mpsc::channel(10);
            tokio::spawn(async move {
                for i in 0..req.a {
                    let res = Res { a: i };
                    let _ = tx.send(res).await;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            });
            Ok(rx)
        }
    }

    #[rpc_func]
    async fn rpc_test_fn(req: Req) -> anyhow::Result<Res> {
        Ok(Res { a: 12 })
    }

    #[tokio::test]
    async fn test_dispatcher_registration() {
        let dis = Dispatcher::new();

        let dt = Test::singleton(Test {});

        dis.write()
            .await
            .register_fn("test".to_owned(), Arc::new(rpc_test_fn));
        dis.write().await.register_struct(dt);

        let res = dis
            .read()
            .await
            .dispatch_rpc("test a=123 b=32.3 c=hey d=[1,2]".as_bytes())
            .await;
        assert!(res.is_ok());

        let res_str = String::from_utf8(res.unwrap()).unwrap();
        assert!(res_str == "{a=12}\n".to_owned());

        let res = dis
            .read()
            .await
            .dispatch_rpc("rpc_fn {a=123 b=32.3 c=hey d=[1,2]}".as_bytes())
            .await;
        assert!(res.is_ok());
        assert!(String::from_utf8(res.unwrap()).unwrap() == "{a=12}\n".to_owned());

        let mut res_chan = dis
            .read()
            .await
            .dispatch_stream("rpc_stream_fn {a=4 b=32.3 c=hey d=[1,2]}".as_bytes())
            .await;
        assert!(res_chan.is_ok());
        let mut vals = Vec::new();
        while let Some(msg) = res_chan.as_mut().unwrap().recv().await {
            vals.push(msg);
        }
        assert_eq!(5, vals.len());
    }
}
