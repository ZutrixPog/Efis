use std::any;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use macros::{rpc_func, rpc_impl, rpc_stream, rpc_struct};
use std::mem::MaybeUninit;
use std::sync::Once;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::time::Duration;
use tracing::warn;

use crate::commands::Command;
use crate::consensus::{CommitEntry, ConsensusHandle};
use crate::efis::types::{GetRes, OkRes, SetReq};
use crate::errors::{DatastoreError, ServiceError};
use crate::pubsub::PubSubGuard;
use crate::rpc::{dispatcher::Dispatcher, RpcStruct};
use crate::rpc::{Deserialize, Serialize};
use crate::store::{DatastoreGuard, Value};

macro_rules! handle_commands {
    ($self_var:ident, $cmd_var:ident, $( $cmd:ident => $func:ident ),* $(,)? ) => {
        match $cmd_var.command {
            $(
                Command::$cmd(req) => {
                    let res = $self_var.$func(req);
                    if let Some(ch) = $self_var.subs.read().await.get(&$cmd_var.index) {
                        let lr = match res {
                            Ok(val) => {
                                let any_val = &val as &dyn std::any::Any;
                                if let Some(okres) = any_val.downcast_ref::<OkRes>() {
                                    LateRes::Ok(okres.clone())
                                } else if let Some(getres) = any_val.downcast_ref::<GetRes<String>>() {
                                    LateRes::Value(getres.clone())
                                } else {
                                    LateRes::Err("internal error".to_string())
                                }
                            }
                            Err(err) => LateRes::Err(err.to_string()),
                        };
                        let _ = ch.send(lr);
                    }
                }
            )*
            Command::Unknown => {}
        }
    };
}

pub mod types {
    use crate::rpc::{Deserialize, Serialize};
    use macros::SerDe;

    #[derive(Clone, SerDe)]
    pub struct OkRes {
        pub status: String,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, SerDe)]
    pub struct SetReq {
        pub key: String,
        pub value: String,
        pub exp: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, SerDe)]
    pub struct GetReq {
        pub key: String,
    }

    #[derive(Clone, SerDe)]
    pub struct GetRes<T: Serialize> {
        pub val: T,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, SerDe)]
    pub struct ExpireReq {
        pub key: String,
        pub duration: u64,
    }

    #[derive(SerDe)]
    pub struct TtlRes {
        pub ttl: String,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, SerDe)]
    pub struct ListReq {
        pub key: String,
        pub values: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, SerDe)]
    pub struct MapReq {
        pub key: String,
        pub score: i64,
        pub value: String,
    }

    #[derive(SerDe)]
    pub struct MapRange {
        pub key: String,
        pub start: usize,
        pub end: usize,
    }

    #[derive(SerDe)]
    pub struct ListRes {
        pub vals: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, SerDe, serde::Serialize, serde::Deserialize)]
    pub struct PubReq {
        pub chan: String,
        pub value: String,
    }

    #[derive(SerDe)]
    pub struct SubReq {
        pub chan: String,
    }
}

#[derive(Clone)]
enum LateRes {
    Ok(OkRes),
    Value(GetRes<String>),
    Err(String),
}

#[rpc_struct]
pub struct Efis {
    store: DatastoreGuard,
    pub pubsub: PubSubGuard,
    cons: Option<&'static ConsensusHandle>,
    cmd_ch_tx: broadcast::Sender<CommitEntry>,
    subs: RwLock<HashMap<usize, broadcast::Sender<LateRes>>>,
}

#[rpc_impl]
impl Efis {
    pub fn new(
        ds: DatastoreGuard,
        ps: PubSubGuard,
        cons: Option<&'static ConsensusHandle>,
    ) -> Self {
        let (tx, _) = broadcast::channel(64);
        let s = Self {
            store: ds,
            pubsub: ps,
            cons,
            cmd_ch_tx: tx,
            subs: RwLock::new(HashMap::new()),
        };

        s
    }

    pub fn singleton(
        ds: DatastoreGuard,
        ps: PubSubGuard,
        cons: Option<&'static ConsensusHandle>,
    ) -> &'static Self {
        static mut SINGLETON: MaybeUninit<Efis> = MaybeUninit::uninit();
        static ONCE: Once = Once::new();

        unsafe {
            ONCE.call_once(|| {
                let singleton = Self::new(ds, ps, cons);
                SINGLETON.write(singleton);
            });

            let s = SINGLETON.assume_init_ref();
            s.process_cmd();
            s
        }
    }

    fn process_cmd(&'static self) {
        tokio::spawn(async move {
            let mut rx = if let Some(cons) = self.cons {
                cons.subscribe()
            } else {
                self.cmd_ch_tx.subscribe()
            };

            loop {
                match rx.recv().await {
                    Ok(cmd) => {
                        handle_commands!(self, cmd,
                            Set       => _set,
                            Delete    => _delete,
                            Increment => _increment,
                            Decrement => _decrement,
                            Expire    => _expire,
                            Lpush     => _lpush,
                            Lpop      => _lpop,
                            Rpush     => _rpush,
                            Rpop      => _rpop,
                            Sadd      => _sadd,
                            Zadd      => _zadd,
                            Publish   => _publish,
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        });
    }

    async fn handle_consensus(&self, cmd: Command) -> LateRes {
        let cons = self.cons.unwrap();
        if let Some(index) = cons.submit(cmd).await {
            let (tx, rx) = broadcast::channel(16);

            self.subs.write().await.insert(index, tx);

            let mut sub = rx.resubscribe();

            let res =
                tokio::time::timeout(Duration::from_secs(5), async { sub.recv().await }).await;

            match res {
                Ok(Ok(lres)) => lres,
                Ok(Err(_)) => LateRes::Err("broadcast recv error".to_string()),
                Err(_) => LateRes::Err("request timed out".to_string()),
            }
        } else {
            LateRes::Err("consensus failed".to_string())
        }
    }

    #[rpc_func]
    async fn set(&'static self, req: types::SetReq) -> anyhow::Result<types::OkRes> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Set(req)).await {
                LateRes::Ok(ok) => Ok(ok),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._set(req)
        }
    }

    fn _set(&self, req: types::SetReq) -> anyhow::Result<types::OkRes> {
        let duration = req.exp.map(Duration::from_secs);
        let res = self
            .store
            .store()
            .set(req.key.clone(), Value::Text(req.value.clone()), duration)
            .map_err(|_| ServiceError::ErrorWrite);

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(OkRes {
                status: "ok".to_string(),
            })
        }
    }

    #[rpc_func]
    fn get(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        let res = self
            .store
            .store()
            .get(req.key.as_str())
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::Text(text) => Ok(text),
                _ => Err(ServiceError::InvalidValueType),
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(types::GetRes { val: res.unwrap() })
        }
    }

    #[rpc_func]
    async fn delete(&'static self, req: types::GetReq) -> anyhow::Result<types::OkRes> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Delete(req)).await {
                LateRes::Ok(ok) => Ok(ok),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._delete(req)
        }
    }

    fn _delete(&self, req: types::GetReq) -> anyhow::Result<types::OkRes> {
        let res = self
            .store
            .store()
            .remove(req.key.as_str())
            .map_err(|_| ServiceError::KeyNotFound)
            .map(|_| ());

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(OkRes {
                status: "ok".to_string(),
            })
        }
    }

    #[rpc_func]
    async fn increment(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Increment(req)).await {
                LateRes::Value(val) => Ok(val),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._increment(req)
        }
    }

    fn _increment(&self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        let mut store = self.store.store();
        let res = store
            .modify(req.key.as_str(), |value| {
                if let Value::Text(val) = value {
                    if let Ok(num) = val.parse::<f64>() {
                        val.clear();
                        val.extend((num + 1.0).to_string().chars())
                    }
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            });

        if res.is_err() {
            return Err(anyhow::anyhow!(res.unwrap_err()));
        }

        let res = store
            .get(req.key.as_str())
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::Text(val) => Ok(val),
                _ => Err(ServiceError::InvalidValueType),
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(types::GetRes { val: res.unwrap() })
        }
    }

    #[rpc_func]
    async fn decrement(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Decrement(req)).await {
                LateRes::Value(val) => Ok(val),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._decrement(req)
        }
    }

    fn _decrement(&self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        let mut store = self.store.store();
        store
            .modify(req.key.as_str(), |value| {
                if let Value::Text(val) = value {
                    if let Ok(num) = val.parse::<f64>() {
                        val.clear();
                        val.extend((num - 1.0).to_string().chars())
                    }
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            })?;

        let res = store
            .get(req.key.as_str())
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::Text(val) => Ok(val),
                _ => Err(ServiceError::InvalidValueType),
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(types::GetRes { val: res.unwrap() })
        }
    }

    #[rpc_func]
    async fn expire(&'static self, req: types::ExpireReq) -> anyhow::Result<OkRes> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Expire(req)).await {
                LateRes::Ok(ok) => Ok(ok),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._expire(req)
        }
    }

    fn _expire(&self, req: types::ExpireReq) -> anyhow::Result<OkRes> {
        let res = self
            .store
            .store()
            .expire(req.key.as_str(), Duration::from_secs(req.duration))
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(OkRes {
                status: "ok".to_string(),
            })
        }
    }

    #[rpc_func]
    fn ttl(&'static self, req: types::GetReq) -> anyhow::Result<types::TtlRes> {
        let res = self
            .store
            .store()
            .ttl(req.key.as_str())
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                DatastoreError::KeyExpired => ServiceError::KeyExpired,
                _ => ServiceError::Other("unkown error".to_owned()),
            })
            .and_then(|ttl| match ttl {
                Some(duration) => Ok(duration.as_secs().to_string()),
                None => Ok("-1".to_string()),
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(types::TtlRes { ttl: res.unwrap() })
        }
    }

    #[rpc_func]
    async fn lpush(&'static self, req: types::ListReq) -> anyhow::Result<OkRes> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Lpush(req)).await {
                LateRes::Ok(ok) => Ok(ok),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._lpush(req)
        }
    }

    fn _lpush(&self, req: types::ListReq) -> anyhow::Result<OkRes> {
        let mut store = self.store.store();
        store
            .set(req.key.clone(), Value::List(VecDeque::new()), None)
            .map_err(|_| ServiceError::ErrorWrite)?;
        let res = store
            .modify(req.key.as_str(), |list| {
                if let Value::List(list_data) = list {
                    for item in req.values.into_iter() {
                        list_data.push_front(item.to_string());
                    }
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(OkRes {
                status: "ok".to_string(),
            })
        }
    }

    #[rpc_func]
    async fn rpush(&'static self, req: types::ListReq) -> anyhow::Result<types::OkRes> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Rpush(req)).await {
                LateRes::Ok(ok) => Ok(ok),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._rpush(req)
        }
    }

    fn _rpush(&self, req: types::ListReq) -> anyhow::Result<types::OkRes> {
        let mut store = self.store.store();
        store
            .set(req.key.clone(), Value::List(VecDeque::new()), None)
            .map_err(|_| ServiceError::ErrorWrite)?;
        let value: Vec<String> = req.values.into_iter().map(|v| v.to_string()).collect();
        let res = store
            .modify(req.key.as_str(), |list| {
                if let Value::List(list_data) = list {
                    list_data.extend(value);
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(OkRes {
                status: "ok".to_string(),
            })
        }
    }

    #[rpc_func]
    async fn lpop(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Lpop(req)).await {
                LateRes::Value(val) => Ok(val),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._lpop(req)
        }
    }

    fn _lpop(&self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        let mut res = Err(ServiceError::ErrorWrite);
        self.store
            .store()
            .modify(req.key.as_str(), |list| {
                if let Value::List(list_data) = list {
                    if let Some(value) = list_data.pop_front() {
                        res = Ok(value);
                    }
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            })?;

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(GetRes { val: res.unwrap() })
        }
    }

    #[rpc_func]
    async fn rpop(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Rpop(req)).await {
                LateRes::Value(val) => Ok(val),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._rpop(req)
        }
    }

    fn _rpop(&self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        let mut res = Err(ServiceError::ErrorWrite);
        self.store
            .store()
            .modify(req.key.as_str(), |list| {
                if let Value::List(list_data) = list {
                    if let Some(value) = list_data.pop_back() {
                        res = Ok(value);
                    }
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            })?;

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(GetRes { val: res.unwrap() })
        }
    }

    #[rpc_func]
    async fn sadd(&'static self, req: types::ListReq) -> anyhow::Result<OkRes> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Sadd(req)).await {
                LateRes::Ok(ok) => Ok(ok),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._sadd(req)
        }
    }

    fn _sadd(&self, req: types::ListReq) -> anyhow::Result<OkRes> {
        let mut store = self.store.store();
        store
            .set(req.key.clone(), Value::Set(HashSet::new()), None)
            .map_err(|_| ServiceError::ErrorWrite)?;
        let value: Vec<String> = req.values.into_iter().map(|v| v.to_string()).collect();
        let res = store
            .modify(req.key.as_str(), |set| {
                if let Value::Set(set_data) = set {
                    set_data.extend(value);
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(OkRes {
                status: "ok".to_string(),
            })
        }
    }

    #[rpc_func]
    fn smembers(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        let res = self
            .store
            .store()
            .get(req.key.as_str())
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::Set(set_data) => Ok(format!("{:?}", set_data)),
                _ => Err(ServiceError::InvalidValueType),
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(GetRes { val: res.unwrap() })
        }
    }

    #[rpc_func]
    async fn zadd(&'static self, req: types::MapReq) -> anyhow::Result<types::OkRes> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Zadd(req)).await {
                LateRes::Ok(ok) => Ok(ok),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._zadd(req)
        }
    }

    fn _zadd(&self, req: types::MapReq) -> anyhow::Result<types::OkRes> {
        let mut store = self.store.store();
        store
            .set(req.key.clone(), Value::SortedSet(BTreeMap::new()), None)
            .map_err(|_| ServiceError::ErrorWrite)?;
        let res = store
            .modify(req.key.as_str(), |zset| {
                if let Value::SortedSet(zset_data) = zset {
                    zset_data.insert(req.score, req.value);
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(OkRes {
                status: "ok".to_string(),
            })
        }
    }

    #[rpc_func]
    fn zrange(&'static self, req: types::MapRange) -> anyhow::Result<types::ListRes> {
        let res = self
            .store
            .store()
            .get(req.key.as_str())
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::SortedSet(zset_data) => {
                    let zset_vec: Vec<(String, i64)> = zset_data
                        .iter()
                        .map(|(k, v)| (v.clone(), k.clone()))
                        .collect();
                    let range = req.start..=req.end;
                    let range_values: Vec<String> =
                        zset_vec[range].iter().map(|(v, _)| v.clone()).collect();
                    Ok(range_values)
                }
                _ => Err(ServiceError::InvalidValueType),
            });

        if res.is_err() {
            Err(anyhow::anyhow!(res.unwrap_err()))
        } else {
            Ok(types::ListRes { vals: res.unwrap() })
        }
    }

    #[rpc_func]
    fn publish(&'static self, req: types::PubReq) -> anyhow::Result<OkRes> {
        if self.cons.is_some() {
            match self.handle_consensus(Command::Publish(req)).await {
                LateRes::Ok(ok) => Ok(ok),
                LateRes::Err(err) => Err(anyhow::format_err!(err)),
                _ => Err(anyhow::format_err!("internal error")),
            }
        } else {
            self._publish(req)
        }
    }

    fn _publish(&'static self, req: types::PubReq) -> anyhow::Result<OkRes> {
        let sent = self.pubsub.ps().publish(req.chan, req.value);
        if sent == 0 && self.cons.is_none() {
            return Err(anyhow::anyhow!(ServiceError::ErrorPublish));
        }
        Ok(types::OkRes {
            status: "ok".to_string(),
        })
    }

    #[rpc_stream]
    fn subscribe(&'static self, req: types::SubReq) -> anyhow::Result<mpsc::Receiver<String>> {
        let (tx, rx) = mpsc::channel(10);
        let mut sub = self.pubsub.ps().subscribe(req.chan);
        tokio::spawn(async move {
            while let Ok(msg) = sub.recv().await {
                let _ = tx.send(msg).await;
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::str::FromStr;

    use super::*;
    use crate::consensus::Consensus;
    use crate::storage::consensus::ConFileStorage;
    use crate::store::DatastoreGuard;

    async fn setup() -> &'static Efis {
        let sguard = DatastoreGuard::new(None, None).await;
        let pguard = PubSubGuard::new();

        let con_storage = ConFileStorage::new(PathBuf::from_str("/var/efis").unwrap());
        let (mut cons, crpc) = Consensus::new(0, con_storage).await;
        tokio::spawn(async move {
            cons.start(vec![]).await;
        });

        Efis::singleton(sguard, pguard, None)
    }

    #[tokio::test]
    async fn test_set() {
        let store_service = setup().await;

        let req = types::SetReq {
            key: "key".to_string(),
            value: "value".to_string(),
            exp: Some(10),
        };
        let res = types::OkRes {
            status: "ok".to_string(),
        };

        let result = store_service.set(req.serialize()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), res.serialize());
    }

    #[tokio::test]
    async fn test_get() {
        let store_service = setup().await;

        let set_req = types::SetReq {
            key: "key_test".to_string(),
            value: "value".to_string(),
            exp: Some(10),
        };
        let get_req = types::GetReq {
            key: "key_test".to_string(),
        };
        let res = types::GetRes {
            val: "value".to_string(),
        };

        let _ = store_service.set(set_req.serialize()).await;
        let result = store_service.get(get_req.serialize()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), res.serialize());
    }

    #[tokio::test]
    async fn test_delete() {
        let store_service = setup().await;

        let set_req = types::SetReq {
            key: "key_delete".to_string(),
            value: "value".to_string(),
            exp: Some(10),
        };
        let del_req = types::GetReq {
            key: "key_delete".to_string(),
        };

        let _ = store_service.set(set_req.serialize()).await;
        let result = store_service.delete(del_req.serialize()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_increment() {
        let store_service = setup().await;

        let set_req = types::SetReq {
            key: "key_incr".to_string(),
            value: "1".to_string(),
            exp: Some(10),
        };
        let incr_req = types::GetReq {
            key: "key_incr".to_string(),
        };
        let incr_res = types::GetRes {
            val: "2".to_string(),
        };

        let _ = store_service.set(set_req.serialize()).await;
        let result = store_service.increment(incr_req.serialize()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), incr_res.serialize());
    }

    #[tokio::test]
    async fn test_decrement() {
        let mut store_service = setup().await;

        let set_req = types::SetReq {
            key: "key_decr".to_string(),
            value: "2".to_string(),
            exp: Some(10),
        };
        let decr_req = types::GetReq {
            key: "key_decr".to_string(),
        };
        let decr_res = types::GetRes {
            val: "1".to_string(),
        };

        let _ = store_service.set(set_req.serialize()).await;
        let result = store_service.decrement(decr_req.serialize()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), decr_res.serialize());
    }

    #[tokio::test]
    async fn test_expire() {
        let store_service = setup().await;

        let set_req = types::SetReq {
            key: "key_exp".to_string(),
            value: "2".to_string(),
            exp: None,
        };
        let exp_req = types::ExpireReq {
            key: "key_exp".to_string(),
            duration: 10,
        };

        let _ = store_service.set(set_req.serialize()).await;
        let result = store_service.expire(exp_req.serialize()).await;
        assert!(result.is_ok())
    }

    #[tokio::test]
    async fn test_ttl() {
        let store_service = setup().await;

        let set_req = types::SetReq {
            key: "key_ttl".to_string(),
            value: "value".to_string(),
            exp: Some(10),
        };
        let ttl_req = types::GetReq {
            key: "key_ttl".to_string(),
        };
        let ttl_res = types::TtlRes {
            ttl: "9".to_string(),
        };

        store_service.set(set_req.serialize()).await;
        let result = store_service.ttl(ttl_req.serialize()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ttl_res.serialize());
    }

    #[tokio::test]
    async fn test_lpush() {
        let store_service = setup().await;

        let vals = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let list_req = types::ListReq {
            key: "key_lpush".to_string(),
            values: vals.clone(),
        };
        let list_res = types::OkRes {
            status: "ok".to_string(),
        };

        let result = store_service.lpush(list_req.serialize()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), list_res.serialize());
    }

    #[tokio::test]
    async fn test_rpush() {
        let store_service = setup().await;

        let vals = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let list_req = types::ListReq {
            key: "key_rpush".to_string(),
            values: vals.clone(),
        };
        let list_res = types::OkRes {
            status: "ok".to_string(),
        };

        let result = store_service.rpush(list_req.serialize()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), list_res.serialize());
    }

    #[tokio::test]
    async fn test_lpop() {
        let store_service = setup().await;

        let vals = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let list_req = types::ListReq {
            key: "key_lpop".to_string(),
            values: vals.clone(),
        };
        let pop_req = types::GetReq {
            key: "key_lpop".to_string(),
        };
        let pop_res = types::GetRes {
            val: "3".to_string(),
        };

        let _ = store_service.lpush(list_req.serialize()).await;
        let result = store_service.lpop(pop_req.serialize()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), pop_res.serialize());
    }

    #[tokio::test]
    async fn test_rpop() {
        let store_service = setup().await;

        let vals = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let list_req = types::ListReq {
            key: "key_rpop".to_string(),
            values: vals.clone(),
        };
        let pop_req = types::GetReq {
            key: "key_rpop".to_string(),
        };
        let pop_res = types::GetRes {
            val: "1".to_string(),
        };

        let _ = store_service.lpush(list_req.serialize()).await;
        let result = store_service.rpop(pop_req.serialize()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), pop_res.serialize());
    }

    #[tokio::test]
    async fn test_sadd() {
        let store_service = setup().await;

        let vals = vec![
            "value1".to_string(),
            "value2".to_string(),
            "value3".to_string(),
        ];
        let list_req = types::ListReq {
            key: "key_sadd".to_string(),
            values: vals.clone(),
        };

        let result = store_service.sadd(list_req.serialize()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_smembers() {
        let store_service = setup().await;

        let vals = vec![
            "value1".to_string(),
            "value2".to_string(),
            "value3".to_string(),
        ];
        let list_req = types::ListReq {
            key: "key_smembers".to_string(),
            values: vals.clone(),
        };
        let sm_req = types::GetReq {
            key: "key_smembers".to_string(),
        };

        let _ = store_service.sadd(list_req.serialize()).await;
        let result = store_service.smembers(sm_req.serialize()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_zadd() {
        let store_service = setup().await;

        let map_req = types::MapReq {
            key: "test_zadd".to_string(),
            value: "value".to_string(),
            score: 12,
        };

        let result = store_service.zadd(map_req.serialize()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_zrange() {
        let store_service = setup().await;

        let _ = store_service._zadd(types::MapReq {
            key: "key_zrng".to_string(),
            score: 3,
            value: "value1".to_string(),
        });
        let _ = store_service._zadd(types::MapReq {
            key: "key_zrng".to_string(),
            score: 2,
            value: "value2".to_string(),
        });
        let _ = store_service._zadd(types::MapReq {
            key: "key_zrng".to_string(),
            score: 1,
            value: "value3".to_string(),
        });

        let req = types::MapRange {
            key: "key_zrng".to_string(),
            start: 0,
            end: 1,
        };

        let result = store_service.zrange(req.serialize()).await;
        assert!(result.is_ok());
        // assert_eq!(result.unwrap(), );
    }

    #[tokio::test]
    async fn test_pubsub() {
        let service = setup().await;
        let key = "ps_key";
        let value = "ps_value";

        let res = service
            .subscribe(
                types::SubReq {
                    chan: key.to_string(),
                }
                .serialize(),
            )
            .await;
        assert!(res.is_ok());
        let mut res_ch = res.unwrap();

        let res = service
            .publish(
                types::PubReq {
                    chan: key.to_string(),
                    value: value.to_string(),
                }
                .serialize(),
            )
            .await;
        assert!(res.is_ok());

        let msg = res_ch.recv().await;
        assert_eq!(msg, Some(value.to_owned() + "\n"));
    }
}
