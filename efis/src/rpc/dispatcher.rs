use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::RpcStruct;

pub type RpcFunc = dyn Fn(String) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>>
    + Send
    + Sync
    + 'static;

pub struct Dispatcher {
    methods: RwLock<HashMap<String, Arc<RpcFunc>>>,
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {
            methods: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register_fn(&self, method: String, rpc_fn: Arc<RpcFunc>) {
        self.methods.write().await.insert(method, rpc_fn);
    }

    // TODO: RpcStruct should be automatically implemented
    pub fn register_struct(&self, st: &'static dyn RpcStruct) {
        st.register_fns(self);
    }

    pub async fn dispatch(&self, req: &[u8]) -> anyhow::Result<Vec<u8>> {
        let req_str = String::from_utf8_lossy(req);
        let mut parts = req_str.split(" ").collect::<Vec<&str>>();
        let method = parts.remove(0);

        let methods = self.methods.read().await;
        let rpc_fn = methods
            .get(method)
            .ok_or_else(|| anyhow::anyhow!("Method not found"))?;

        let response = rpc_fn(parts.join(" ")).await?;

        Ok(response.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::SerDe;
    use crate::rpc::{Deserialize, Serialize};
    use macros::{rpc_func, SerDe};
    use std::mem::MaybeUninit;
    use std::sync::Once;

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Req {
        a: i32,
        b: f64,
        c: String,
    }

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Res {
        a: i32,
    }

    struct Test {}

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
    }

    impl RpcStruct for Test {
        fn register_fns(&'static self, dispatcher: &super::Dispatcher) {
            dispatcher.register_fn(
                "test2".to_owned(),
                Arc::new(move |req: String| Box::pin(async move { self.rpc_fn(req).await })),
            );
        }
    }

    #[rpc_func]
    async fn rpc_test_fn(req: Req) -> anyhow::Result<Res> {
        println!("shit");
        Ok(Res { a: 12 })
    }

    #[tokio::test]
    async fn test_dispatcher_registration() {
        let dis = Dispatcher::new();

        let dt = Test::singleton(Test {});

        dis.register_fn("test".to_owned(), Arc::new(rpc_test_fn));
        dis.register_struct(dt);

        let res = dis.dispatch("test 123 32.3 hey".as_bytes()).await;
        assert!(res.is_ok());
        assert!(String::from_utf8(res.unwrap()).unwrap() == "12".to_owned());

        let res = dis.dispatch("test2 123 32.3 hey".as_bytes()).await;
        assert!(res.is_ok());
        assert!(String::from_utf8(res.unwrap()).unwrap() == "12".to_owned());
    }
}
