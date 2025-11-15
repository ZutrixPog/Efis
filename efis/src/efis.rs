use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet, VecDeque};

use macros::{rpc_func, rpc_impl, rpc_stream, rpc_struct};
use std::mem::MaybeUninit;
use std::sync::{Arc, Once};
use tokio::sync::mpsc;
use tokio::time::Duration;

use crate::consensus::Consensus;
use crate::efis::types::{GetRes, OkRes};
use crate::errors::{DatastoreError, ServiceError};
use crate::pubsub::PubSubGuard;
use crate::rpc::{dispatcher::Dispatcher, RpcStruct};
use crate::rpc::{Deserialize, Serialize};
use crate::store::{DatastoreGuard, Value};

mod types {
    use crate::rpc::{Deserialize, Serialize};
    use macros::SerDe;

    #[derive(SerDe)]
    pub struct OkRes {
        pub status: String,
    }

    #[derive(SerDe)]
    pub struct SetReq {
        pub key: String,
        pub value: String,
        pub exp: Option<u64>,
    }

    #[derive(SerDe)]
    pub struct GetReq {
        pub key: String,
    }

    #[derive(SerDe)]
    pub struct GetRes<T: Serialize> {
        pub val: T,
    }

    #[derive(SerDe)]
    pub struct ExpireReq {
        pub key: String,
        pub duration: u64,
    }

    #[derive(SerDe)]
    pub struct TtlRes {
        pub ttl: String,
    }

    #[derive(SerDe)]
    pub struct ListReq {
        pub key: String,
        pub values: Vec<String>,
    }

    #[derive(SerDe)]
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

    #[derive(SerDe)]
    pub struct PubReq {
        pub chan: String,
        pub value: String,
    }

    #[derive(SerDe)]
    pub struct SubReq {
        pub chan: String,
    }
}

//#[derive(Debug)]
#[rpc_struct]
pub struct Efis {
    store: DatastoreGuard,
    pub pubsub: PubSubGuard,
}

#[rpc_impl]
impl Efis {
    pub fn new(ds: DatastoreGuard, ps: PubSubGuard) -> Self {
        Self {
            store: ds,
            pubsub: ps,
        }
    }

    pub fn singleton(ds: DatastoreGuard, ps: PubSubGuard) -> &'static Self {
        static mut SINGLETON: MaybeUninit<Efis> = MaybeUninit::uninit();
        static ONCE: Once = Once::new();

        unsafe {
            ONCE.call_once(|| {
                let singleton = Self::new(ds, ps);
                SINGLETON.write(singleton);
            });

            SINGLETON.assume_init_ref()
        }
    }

    #[rpc_func]
    fn set(&'static self, req: types::SetReq) -> anyhow::Result<types::OkRes> {
        let duration = req.exp.map(Duration::from_secs);
        let res = self
            .store
            .store()
            .set(req.key, Value::Text(req.value), duration)
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
    fn delete(&'static self, req: types::GetReq) -> anyhow::Result<types::OkRes> {
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
    fn increment(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
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
    fn decrement(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
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
    fn expire(&'static self, req: types::ExpireReq) -> anyhow::Result<OkRes> {
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
    fn lpush(&'static self, req: types::ListReq) -> anyhow::Result<OkRes> {
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
    fn rpush(&'static self, req: types::ListReq) -> anyhow::Result<types::OkRes> {
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
    fn lpop(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
        let mut res = Err(ServiceError::ErrorWrite);
        self.store
            .store()
            .modify(req.key.as_str(), |list| {
                if let Value::List(list_data) = list {
                    if let Some(value) = list_data.pop_front() {
                        println!("{}", value.clone());
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
    fn rpop(&'static self, req: types::GetReq) -> anyhow::Result<types::GetRes<String>> {
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
    fn sadd(&'static self, req: types::ListReq) -> anyhow::Result<OkRes> {
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
    fn zadd(&'static self, req: types::MapReq) -> anyhow::Result<types::OkRes> {
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
        let sent = self.pubsub.ps().publish(req.chan, req.value);
        if sent > 0 {
            Ok(types::OkRes {
                status: "ok".to_string(),
            })
        } else {
            Err(anyhow::anyhow!(ServiceError::ErrorPublish))
        }
    }

    #[rpc_stream]
    fn subscribe(&'static self, req: types::SubReq) -> anyhow::Result<mpsc::Receiver<String>> {
        let (tx, rx) = mpsc::channel(10);
        let mut sub = self.pubsub.ps().subscribe(req.chan);
        tokio::spawn(async move {
            while let Ok(mut msg) = sub.recv().await {
                tx.send(msg).await;
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
    use crate::storage::consensus::ConFileStorage;
    use crate::store::DatastoreGuard;
    use tokio::sync::mpsc;

    async fn setup() -> &'static Efis {
        let sguard = DatastoreGuard::new(None, None).await;
        let pguard = PubSubGuard::new();

        let con_storage = ConFileStorage::new(PathBuf::from_str("/var/efis").unwrap());
        let (commit_chan_tx, commit_chan_rx) = mpsc::channel(1024);
        let (cons, crpc) = Consensus::singleton(0, con_storage).await;
        tokio::spawn(async move {
            cons.start(vec![], commit_chan_tx).await;
        });

        Efis::singleton(sguard, pguard)
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

        let _ = store_service
            .zadd(
                types::MapReq {
                    key: "key_zrng".to_string(),
                    score: 3,
                    value: "value1".to_string(),
                }
                .serialize(),
            )
            .await;
        let _ = store_service
            .zadd(
                types::MapReq {
                    key: "key_zrng".to_string(),
                    score: 2,
                    value: "value2".to_string(),
                }
                .serialize(),
            )
            .await;
        let _ = store_service
            .zadd(
                types::MapReq {
                    key: "key_zrng".to_string(),
                    score: 1,
                    value: "value3".to_string(),
                }
                .serialize(),
            )
            .await;

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
