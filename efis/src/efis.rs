use std::collections::{BTreeMap, HashSet, VecDeque};

use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::time::Duration;

use crate::consensus::{Consensus, RequestVote};
use crate::errors::{DatastoreError, ServiceError};
use crate::parser::{parse_command, EfisCommand};
use crate::pubsub::PubSub;
use crate::store::{Datastore, Value};

//#[derive(Debug)]
pub struct Efis {
    store: Datastore,
    pub pubsub: PubSub,
    cons: Arc<Consensus>,
}

impl Efis {
    pub fn new(ds: Datastore, ps: PubSub, cons: Arc<Consensus>) -> Self {
        Self {
            store: ds,
            pubsub: ps,
            cons,
        }
    }

    pub async fn process_cmd(
        &mut self,
        cmd_str: &str,
        res_chan: Sender<String>,
    ) -> Result<String, ServiceError> {
        let command =
            parse_command(cmd_str).unwrap_or(("unknown", EfisCommand::Unknown("unknown command")));

        if let EfisCommand::Subscribe(chan) = command.1 {
            let mut sub = self.pubsub.subscribe(chan.to_owned());
            tokio::spawn(async move {
                while let Ok(mut msg) = sub.recv().await {
                    if msg.contains("unsub") {
                        break;
                    }
                    msg.push('\n');

                    let _ = res_chan.send(msg).await;
                }
            });

            return Ok("PS".to_string());
        }

        let ok_res = Ok("OK".to_string());
        match command.1 {
            EfisCommand::Set(key, value, expiration) => {
                self.set(key, value, expiration).and(ok_res)
            }
            EfisCommand::Get(key) => self.get(key),
            EfisCommand::Del(key) => self.delete(key).and(ok_res),
            EfisCommand::Incr(key) => self.increment(key).and(ok_res),
            EfisCommand::Decr(key) => self.decrement(key).and(ok_res),
            EfisCommand::Expire(key, expiration) => self.expire(key, expiration).and(ok_res),
            EfisCommand::TTL(key) => self.ttl(key),
            EfisCommand::LPush(key, values) => self.lpush(key, values).and(ok_res),
            EfisCommand::RPush(key, values) => self.rpush(key, values).and(ok_res),
            EfisCommand::LPop(key) => self.lpop(key),
            EfisCommand::RPop(key) => self.rpop(key),
            EfisCommand::SAdd(key, members) => self.sadd(key, members).and(ok_res),
            EfisCommand::SMembers(key) => self.smembers(key),
            EfisCommand::ZAdd(key, member, score) => self.zadd(key, member, score).and(ok_res),
            EfisCommand::ZRange(key, start, stop) => self.zrange(key, start, stop),
            EfisCommand::Publish(channel, message) => self.publish(channel, message).and(ok_res),
            EfisCommand::AppendEntries(ae) => self
                .cons
                .append_entries(ae)
                .await
                .map(|reply| {
                    format!(
                        "{} {} {} {}",
                        reply.term, reply.success, reply.conflict_index, reply.conflict_term
                    )
                })
                .map_err(|err| ServiceError::Other(err.to_string())),
            EfisCommand::RequestVote(term, candidate_id, last_log_index, last_long_term) => self
                .cons
                .request_vote(RequestVote {
                    term: term as usize,
                    candidate_id: candidate_id as usize,
                    last_log_index: last_log_index as usize,
                    last_long_term: last_long_term as usize,
                })
                .await
                .map(|reply| format!("{} {}", reply.term, reply.voted))
                .map_err(|err| ServiceError::Other(err.to_string())),
            _ => Err(ServiceError::UnknownCommand),
        }
    }

    fn set(&mut self, key: &str, value: &str, exp: Option<u64>) -> Result<(), ServiceError> {
        let duration = exp.map(Duration::from_secs);
        self.store
            .set(key.to_string(), Value::Text(value.to_string()), duration)
            .map_err(|_| ServiceError::ErrorWrite)
    }

    fn get(&self, key: &str) -> Result<String, ServiceError> {
        self.store
            .get(key)
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::Text(text) => Ok(text),
                _ => Err(ServiceError::InvalidValueType),
            })
    }

    fn delete(&mut self, key: &str) -> Result<(), ServiceError> {
        self.store
            .remove(key)
            .map_err(|_| ServiceError::KeyNotFound)
            .map(|_| ())
    }

    fn increment(&mut self, key: &str) -> Result<String, ServiceError> {
        self.store
            .modify(key, |value| {
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
            })?;

        self.store
            .get(key)
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::Text(val) => Ok(val),
                _ => Err(ServiceError::InvalidValueType),
            })
    }

    fn decrement(&mut self, key: &str) -> Result<String, ServiceError> {
        self.store
            .modify(key, |value| {
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

        self.store
            .get(key)
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::Text(val) => Ok(val),
                _ => Err(ServiceError::InvalidValueType),
            })
    }

    fn expire(&mut self, key: &str, duration: u64) -> Result<(), ServiceError> {
        self.store
            .expire(key, Duration::from_secs(duration))
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            })
    }

    fn ttl(&self, key: &str) -> Result<String, ServiceError> {
        self.store
            .ttl(key)
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                DatastoreError::KeyExpired => ServiceError::KeyExpired,
                _ => ServiceError::Other("unkown error".to_owned()),
            })
            .and_then(|ttl| match ttl {
                Some(duration) => Ok(duration.as_secs().to_string()),
                None => Ok("-1".to_string()),
            })
    }

    fn lpush(&mut self, key: &str, values: Vec<&str>) -> Result<(), ServiceError> {
        self.store
            .set(key.to_string(), Value::List(VecDeque::new()), None)
            .map_err(|_| ServiceError::ErrorWrite)?;
        self.store
            .modify(key, |list| {
                if let Value::List(list_data) = list {
                    for item in values.into_iter() {
                        list_data.push_front(item.to_string());
                    }
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            })
    }

    fn rpush(&mut self, key: &str, values: Vec<&str>) -> Result<(), ServiceError> {
        self.store
            .set(key.to_string(), Value::List(VecDeque::new()), None)
            .map_err(|_| ServiceError::ErrorWrite)?;
        let value: Vec<String> = values.into_iter().map(|v| v.to_string()).collect();
        self.store
            .modify(key, |list| {
                if let Value::List(list_data) = list {
                    list_data.extend(value);
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            })
    }

    fn lpop(&mut self, key: &str) -> Result<String, ServiceError> {
        let mut res = Err(ServiceError::ErrorWrite);
        self.store
            .modify(key, |list| {
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

        res
    }

    fn rpop(&mut self, key: &str) -> Result<String, ServiceError> {
        let mut res = Err(ServiceError::ErrorWrite);
        self.store
            .modify(key, |list| {
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

        res
    }

    fn sadd(&mut self, key: &str, values: Vec<&str>) -> Result<(), ServiceError> {
        self.store
            .set(key.to_string(), Value::Set(HashSet::new()), None)
            .map_err(|_| ServiceError::ErrorWrite)?;
        let value: Vec<String> = values.into_iter().map(|v| v.to_string()).collect();
        self.store
            .modify(key, |set| {
                if let Value::Set(set_data) = set {
                    set_data.extend(value);
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            })
    }

    fn smembers(&self, key: &str) -> Result<String, ServiceError> {
        self.store
            .get(key)
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::Set(set_data) => Ok(format!("{:?}", set_data)),
                _ => Err(ServiceError::InvalidValueType),
            })
    }

    fn zadd(&mut self, key: &str, score: &str, value: &str) -> Result<(), ServiceError> {
        self.store
            .set(key.to_string(), Value::SortedSet(BTreeMap::new()), None)
            .map_err(|_| ServiceError::ErrorWrite)?;
        self.store
            .modify(key, |zset| {
                if let Value::SortedSet(zset_data) = zset {
                    if let Ok(s) = score.parse() {
                        zset_data.insert(s, value.to_string());
                    }
                }
            })
            .map_err(|err| match err {
                DatastoreError::KeyNotFound => ServiceError::KeyNotFound,
                _ => ServiceError::ErrorWrite,
            })
    }

    fn zrange(&self, key: &str, start: u64, end: u64) -> Result<String, ServiceError> {
        self.store
            .get(key)
            .ok_or(ServiceError::KeyNotFound)
            .and_then(|value| match value {
                Value::SortedSet(zset_data) => {
                    let zset_vec: Vec<(String, i64)> = zset_data
                        .iter()
                        .map(|(k, v)| (v.clone(), k.clone()))
                        .collect();
                    let range = start as usize..=end as usize;
                    let range_values: Vec<String> =
                        zset_vec[range].iter().map(|(v, _)| v.clone()).collect();
                    Ok(format!("{:?}", range_values))
                }
                _ => Err(ServiceError::InvalidValueType),
            })
    }

    fn publish(&mut self, key: &str, value: &str) -> Result<(), ServiceError> {
        let sent = self.pubsub.publish(key.to_string(), value.to_owned());
        if sent > 0 {
            Ok(())
        } else {
            Err(ServiceError::ErrorPublish)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::str::FromStr;

    use super::*;
    use crate::pubsub::PubSubGuard;
    use crate::storage::consensus::ConFileStorage;
    use crate::store::DatastoreGuard;
    use tokio::sync::{mpsc, Notify};

    async fn setup() -> Efis {
        let guard = DatastoreGuard::new(None, None).await;
        let store = guard.store();
        let pguard = PubSubGuard::new();
        let pubsub = pguard.ps();

        let con_storage = ConFileStorage::new(PathBuf::from_str("/var/efis").unwrap());
        let ready_ntf = Notify::new();
        let (_, commit_chan_rx) = mpsc::channel(1024);
        let cons = Consensus::new(0, vec![], con_storage, ready_ntf, commit_chan_rx).await;

        Efis::new(store, pubsub, Arc::clone(&cons))
    }

    #[tokio::test]
    async fn test_set() {
        let mut store_service = setup().await;
        let result = store_service.set("key", "value", Some(10));
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn test_get() {
        let mut store_service = setup().await;
        store_service.set("key", "value", Some(10)).unwrap();
        let result = store_service.get("key");
        assert_eq!(result, Ok("value".to_string()));
    }

    #[tokio::test]
    async fn test_delete() {
        let mut store_service = setup().await;
        store_service.set("key", "value", Some(10)).unwrap();
        let result = store_service.delete("key");
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn test_increment() {
        let mut store_service = setup().await;
        store_service.set("key", "1", Some(10)).unwrap();
        let result = store_service.increment("key");
        assert_eq!(result, Ok("2".to_string()));
    }

    #[tokio::test]
    async fn test_decrement() {
        let mut store_service = setup().await;
        store_service.set("key", "2", Some(10)).unwrap();
        let result = store_service.decrement("key");
        assert_eq!(result, Ok("1".to_string()));
    }

    #[tokio::test]
    async fn test_expire() {
        let mut store_service = setup().await;
        store_service.set("key", "value", None).unwrap();
        let result = store_service.expire("key", 10);
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn test_ttl() {
        let mut store_service = setup().await;
        store_service.set("key", "value", Some(10)).unwrap();
        let result = store_service.ttl("key");
        assert_eq!(result, Ok("9".to_string()));
    }

    #[tokio::test]
    async fn test_lpush() {
        let mut store_service = setup().await;
        let result = store_service.lpush("key", vec!["1", "2", "3"]);
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn test_rpush() {
        let mut store_service = setup().await;
        store_service.set("key", "value", None).unwrap();
        let result = store_service.rpush("key", vec!["1", "2", "3"]);
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn test_lpop() {
        let mut store_service = setup().await;
        store_service.lpush("key", vec!["1", "2", "3"]).unwrap();
        let result = store_service.lpop("key");
        assert_eq!(result, Ok("3".to_string()));
    }

    #[tokio::test]
    async fn test_rpop() {
        let mut store_service = setup().await;
        store_service.lpush("key", vec!["1", "2", "3"]).unwrap();
        let result = store_service.rpop("key");
        assert_eq!(result, Ok("1".to_string()));
    }

    #[tokio::test]
    async fn test_sadd() {
        let mut store_service = setup().await;
        let result = store_service.sadd("key", vec!["value1", "value2", "value3"]);
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn test_smembers() {
        let mut store_service = setup().await;
        store_service
            .sadd("key", vec!["value1", "value2", "value3"])
            .unwrap();
        let result = store_service.smembers("key");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_zadd() {
        let mut store_service = setup().await;
        let result = store_service.zadd("key", "12", "value");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_zrange() {
        let mut store_service = setup().await;
        store_service.zadd("key", "3", "value1").unwrap();
        store_service.zadd("key", "2", "value2").unwrap();
        store_service.zadd("key", "1", "value3").unwrap();
        let result = store_service.zrange("key", 0, 1);
        assert_eq!(result, Ok("[\"value3\", \"value2\"]".to_string()));
    }

    // #[tokio::test]
    // async fn test_pubsub() {
    //     let mut service = setup().await;
    //     let key = "test_key";
    //     let value = "test_value";

    //     service.subscribe(key, |msg| {
    //         assert_eq!(msg, value.to_owned());
    //     });

    //     service.publish(key, value);

    // }
}
