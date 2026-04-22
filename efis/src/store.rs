use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::cmp::PartialEq;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::convert::From;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};
use tracing::{error, info};

use crate::serializer::{decode, encode};
use crate::storage::backup::FileBackupRepo;
use crate::vector::flat::FlatIndex;

const PATH: &str = "./backup";
pub type Key = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Text(String),
    List(VecDeque<String>),
    Set(HashSet<String>),
    SortedSet(BTreeMap<i64, String>),
    Vector(FlatIndex),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct Item {
    value: Value,
    expiry: Option<SystemTime>,
}

pub struct DatastoreGuard {
    store: Arc<Datastore>,
    interval: Option<Duration>,
    path: Option<String>,
    notify_shutdown: Option<broadcast::Sender<()>>,
}

impl DatastoreGuard {
    pub async fn new(interval: Option<Duration>, path: Option<String>) -> Self {
        let repo =
            FileBackupRepo::new(Path::new(&path.clone().unwrap_or(PATH.to_string())).to_path_buf());

        let mut guard = if let Ok(data) = repo.retrieve().await {
            info!("reading data from backup.");
            let mut g = DatastoreGuard::from(data);
            g.interval = interval;
            g.path = path;
            g
        } else {
            Self {
                store: Arc::new(Datastore::new()),
                interval,
                path,
                notify_shutdown: None,
            }
        };

        if guard.interval.is_some() && guard.path.is_some() {
            guard.run_backup();
        }
        guard
    }

    pub fn store(&self) -> Arc<Datastore> {
        self.store.clone()
    }

    pub fn run_backup(&mut self) {
        if self.interval.is_none() {
            return;
        }

        let dur = self.interval.unwrap().clone();
        let data = self.store();
        let path = self.path.clone().unwrap();

        let (notify, mut receiver) = broadcast::channel(1);
        self.notify_shutdown = Some(notify);
        tokio::spawn(async move {
            let repo = FileBackupRepo::new(Path::new(&path).to_path_buf());
            let mut inter = interval(dur);

            loop {
                tokio::select! {
                    _ = receiver.recv() => {
                        info!("persisting data before exiting...");
                        backup(&data, &repo).await;
                        return;
                    },
                    _ = inter.tick() => {
                        backup(&data, &repo).await;
                    }
                }
            }
        });
    }
}

async fn backup(data: &Datastore, repo: &FileBackupRepo) {
    // info!("backup data persisted on disk.");
    let data = data.encode().await.unwrap();
    if let Err(err) = repo.save(data).await {
        error!("backup service stopped: {}", err.to_string());
        return;
    }
}

impl Drop for DatastoreGuard {
    fn drop(&mut self) {
        self.store.shutdown_purge_task();
        if let Some(sender) = self.notify_shutdown.clone() {
            let _ = sender.send(());
        }
    }
}

pub struct Datastore {
    data: DashMap<String, Item>,
}

impl Datastore {
    pub fn new() -> Self {
        Self {
            data: DashMap::new(),
        }
    }

    async fn encode(&self) -> anyhow::Result<Vec<u8>> {
        Ok(Vec::new())
        // encode(self.data.clone()).map_err(|_| anyhow::format_err!("couldn't encode"))
    }

    pub fn shutdown_purge_task(&self) {}
}

impl Datastore {
    pub async fn set(
        &self,
        key: String,
        value: Value,
        expiry: Option<Duration>,
    ) -> anyhow::Result<()> {
        let item = Item {
            value: value,
            expiry: expiry.map(|d| SystemTime::now() + d),
        };
        self.data.insert(key, item);
        Ok(())
    }

    pub fn get_sync(&self, key: &str) -> Option<Value> {
        if let Some(item) = self.data.get(key) {
            if let Some(expiry) = item.expiry {
                if expiry <= SystemTime::now() {
                    self.data.remove(key);
                    return None;
                }
            }

            Some(item.value.clone())
        } else {
            None
        }
    }

    pub async fn get(&self, key: &str) -> Option<Value> {
        self.get_sync(key)
    }

    pub async fn remove(&self, key: &str) -> anyhow::Result<()> {
        if self.data.remove(key).is_some() {
            Ok(())
        } else {
            Err(anyhow::format_err!("key not found"))
        }
    }

    pub async fn expire(&self, key: &str, duration: Duration) -> anyhow::Result<()> {
        if let Some(mut item) = self.data.get_mut(key) {
            item.expiry = Some(SystemTime::now() + duration);
            Ok(())
        } else {
            Err(anyhow::format_err!("key not found"))
        }
    }

    pub async fn ttl(&self, key: &str) -> anyhow::Result<Option<Duration>> {
        if let Some(item) = self.data.get(key) {
            if let Some(expiry) = item.expiry {
                let now = SystemTime::now();
                if now >= expiry {
                    self.data.remove(key);
                    return Err(anyhow::format_err!("key expired"));
                }
                if let Ok(duration) = expiry.duration_since(now) {
                    Ok(Some(duration))
                } else {
                    Ok(None)
                }
            } else {
                Ok(None)
            }
        } else {
            Err(anyhow::format_err!("key not found"))
        }
    }

    pub async fn modify<F>(&self, key: &str, modifier: F) -> anyhow::Result<()>
    where
        F: FnOnce(&mut Value) -> anyhow::Result<()>,
    {
        if let Some(mut item) = self.data.get_mut(key) {
            let value = &mut item.value;
            modifier(value)?;
            Ok(())
        } else {
            Err(anyhow::format_err!("key not found"))
        }
    }
}

impl From<Vec<u8>> for DatastoreGuard {
    fn from(value: Vec<u8>) -> Self {
        // let decoded = decode(&value).unwrap();
        let decoded = DashMap::new();

        DatastoreGuard {
            store: Arc::new(Datastore { data: decoded }),
            interval: None,
            path: None,
            notify_shutdown: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_set_and_get() {
        let guard = DatastoreGuard::new(None, None).await;
        let datastore = guard.store();

        let mut list = VecDeque::new();
        list.push_back("hello".to_owned());

        let mut set = HashSet::new();
        set.insert("6.13".to_owned());

        let mut sorted = BTreeMap::new();
        sorted.insert(1, "geez".to_owned());

        let vals = vec![
            ("key1".to_owned(), Value::Text("value1".to_owned()), None),
            (
                "key2".to_owned(),
                Value::Text("value2".to_owned()),
                Some(Duration::from_secs(2)),
            ),
            ("key3".to_owned(), Value::List(list.clone()), None),
            ("key4".to_owned(), Value::Set(set.clone()), None),
            ("Key5".to_owned(), Value::SortedSet(sorted.clone()), None),
        ];

        for (key, value, duration) in vals {
            let _ = datastore.set(key, value, duration);
        }

        let cases = vec![
            ("key1", Some(Value::Text("value1".to_owned()))),
            ("key2", None),
            ("key3", Some(Value::List(list))),
            ("key4", Some(Value::Set(set))),
            ("Key5", Some(Value::SortedSet(sorted))),
        ];

        std::thread::sleep(Duration::from_secs(4));

        for (key, res) in cases {
            assert_eq!(datastore.get(key).await, res);
        }
    }

    #[tokio::test]
    async fn test_remove() {
        let guard = DatastoreGuard::new(None, None).await;
        let datastore = guard.store();

        let _ = datastore.set("key1".to_owned(), Value::Text("value1".to_owned()), None);

        let result = datastore.remove("key1").await;
        assert!(result.is_ok());

        let result = datastore.remove("non_existent_key").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_expire_and_ttl() {
        let guard = DatastoreGuard::new(None, None).await;
        let datastore = guard.store();

        let _ = datastore.set(
            "key1".to_owned(),
            Value::Text("value1".to_owned()),
            Some(Duration::from_secs(2)),
        );
        let _ = datastore
            .set("key2".to_owned(), Value::Text("value2".to_owned()), None)
            .await;

        assert!(datastore.ttl("key1").await.is_ok());
        assert!(datastore.ttl("key2").await.is_ok());

        std::thread::sleep(Duration::from_secs(4));

        assert!(datastore.ttl("key1").await.is_err());
        assert!(datastore.get("key1").await.is_none());
    }

    #[tokio::test]
    async fn test_modify_existing_key() {
        let guard = DatastoreGuard::new(None, None).await;
        let datastore = guard.store();

        let _ = datastore.set("key".to_owned(), Value::Text("value".to_owned()), None);

        let res = datastore
            .modify("key", |value| {
                if let Value::Text(ref mut v) = value {
                    *v = "new_value".to_owned();
                }
                Ok(())
            })
            .await;

        assert!(res.is_ok());
        assert_eq!(
            datastore.get("key").await,
            Some(Value::Text("new_value".to_owned()))
        );
    }

    #[tokio::test]
    async fn test_encode_decode() {
        let guard = DatastoreGuard::new(None, None).await;
        let datastore = guard.store();

        let (key, value) = ("key", Value::Text("value".to_owned()));
        let (key1, value1) = ("key1", Value::Text("value1".to_owned()));

        let _ = datastore.set(key.to_owned(), value.clone(), None);
        let _ = datastore.set(key1.to_owned(), value1.clone(), None);

        let mut test_data = HashMap::new();
        test_data.insert(
            key.to_owned(),
            Item {
                value: value,
                expiry: None,
            },
        );
        test_data.insert(
            key1.to_owned(),
            Item {
                value: value1,
                expiry: None,
            },
        );

        let encoded = datastore.encode().await;
        assert!(encoded.is_ok());

        let encoded = encoded.unwrap();
        let decoded = decode::<HashMap<String, Item>>(&encoded);
        assert!(decoded.is_ok());

        let decoded = decoded.unwrap();
        assert_eq!(test_data, decoded);
    }

    #[tokio::test]
    async fn test_run_backup() {
        let backup_interval = Duration::from_secs(1);
        let data = String::from("data");

        let guard = DatastoreGuard::new(Some(backup_interval), Some(PATH.to_owned())).await;
        let store = guard.store();
        let _ = store.set("data".to_owned(), Value::Text(data), None);

        sleep(Duration::from_secs(2)).await;

        // Assert that the backup file was created
        let backup_file_exists = std::path::Path::new("./backup/backup.efs").exists();
        assert!(backup_file_exists);
    }
}
