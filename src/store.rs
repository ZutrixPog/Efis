use tokio::time::{Duration, Instant};
use std::sync::{Arc, Mutex};
use std::convert::From;
use std::thread;
use std::collections::{VecDeque, HashSet, BTreeMap, HashMap};
use std::cmp::PartialEq;
use std::path::Path;
use tracing::{info, error};
use serde::{Deserialize, Serialize};

use crate::errors::*;
use crate::persist::repo::FileBackupRepo;
use crate::serializer::{encode, decode};


const PATH: &str = "./backup";
pub type Key = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Text(String),
    List(VecDeque<String>),
    Set(HashSet<String>),
    SortedSet(BTreeMap<i64, String>),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct Item {
    value: Value,
    #[serde(skip)]
    expiry: Option<Instant>,
}

#[derive(Debug)]
pub struct DatastoreGuard {
    store: Datastore,
    interval: Option<Duration>,
    path: Option<String>,
}

impl DatastoreGuard {
    pub async fn new(interval: Option<Duration>, path: Option<String>) -> Self {
        let repo = FileBackupRepo::new(Path::new(&path.clone().unwrap_or(PATH.to_string())).to_path_buf());
        
        let guard = if let Ok(data) = repo.retrieve().await {
            info!("reading data from backup.");
            let mut g = DatastoreGuard::from(data);
            g.interval = interval;
            g.path = path;
            g
        } else {
            Self { store: Datastore::new(), interval, path }
        };
        
        if guard.interval.is_some() {
            guard.run_backup();
        }
        guard
    }
    
    pub fn store(&self) -> Datastore {
        self.store.clone()
    }

    pub fn run_backup(&self) {
        if self.interval.is_none() {
            return;
        }

        let dur = self.interval.unwrap().clone();
        let data = self.store();
        let path = self.path.clone().unwrap();
        tokio::spawn(async move {
            let repo = FileBackupRepo::new(Path::new(&path).to_path_buf());
            
            loop {
                thread::sleep(dur);
                
                info!("backup data persisted on disk.");
                let data = data.encode().unwrap();
                if let Err(err) = repo.save(data).await {
                    error!("backup service stopped: {}", err.to_string());
                    return;
                }
            }
        });
    }
}

impl Drop for DatastoreGuard {
    fn drop(&mut self) {
        self.store.shutdown_purge_task();
    }
}

#[derive(Debug, Clone)]
pub struct Datastore {
    data: Arc<Mutex<HashMap<String, Item>>>,
}

impl Datastore {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new()))
        }
    }

    fn encode(&self) -> Result<Vec<u8>, DatastoreError> {
        let data = self.data.lock().unwrap();
        encode(data.clone())
            .map_err(|_| DatastoreError::Other("couldn't encode".to_string()))
    }
    
    pub fn shutdown_purge_task(&self) {
        let state = self.data.lock().unwrap();
        drop(state);
    }
}

impl Datastore {
    pub fn set(&mut self, key: String, value: Value, expiry: Option<Duration>) -> Result<(), DatastoreError> {
        let item = Item {
            value: value,
            expiry: expiry.map(|d| Instant::now() + d),
        };
        let mut data = self.data.lock().unwrap();
        data.entry(key).or_insert(item);
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let mut data = self.data.lock().unwrap();
        if let Some(item) = data.get(key) {
            if let Some(expiry) = item.expiry {
                if expiry <= Instant::now() {
                    data.remove(key);
                    return None;
                }
            }
            
            Some(item.value.clone())
        } else {
            None
        }
    }

    pub fn remove(&mut self, key: &str) -> Result<(), DatastoreError> {
        let mut data = self.data.lock().unwrap();
        if data.remove(key).is_some() {
            Ok(())
        } else {
            Err(DatastoreError::KeyNotFound)
        }
    }

    pub fn expire(&mut self, key: &str, duration: Duration) -> Result<(), DatastoreError> {
        let mut data = self.data.lock().unwrap();
        if let Some(item) = data.get_mut(key) {
            item.expiry = Some(Instant::now() + duration);
            Ok(())
        } else {
            Err(DatastoreError::KeyNotFound)
        }
    }

    pub fn ttl(&self, key: &str) -> Result<Option<Duration>, DatastoreError> {
        let mut data = self.data.lock().unwrap();
        if let Some(item) = data.get(key) {
            if let Some(expiry) = item.expiry {
                let now = Instant::now();
                if now >= expiry {
                    data.remove(key);
                    return Err(DatastoreError::KeyExpired);
                }
                Ok(Some(expiry - now))
            } else {
                Ok(None)
            }
        } else {
            Err(DatastoreError::KeyNotFound)
        }
    }

    pub fn modify<F>(&mut self, key: &str, modifier: F) -> Result<(), DatastoreError>
    where
        F: FnOnce(&mut Value),
    {
        let mut data = self.data.lock().unwrap();
        if let Some(item) = data.get_mut(key) {
            let value = &mut item.value;
            modifier(value);
            Ok(())
        } else {
            Err(DatastoreError::KeyNotFound)
        }
    }

}

impl From<Vec<u8>> for DatastoreGuard {
    fn from(value: Vec<u8>) -> Self {
        let decoded = decode(&value).unwrap();

        DatastoreGuard {
            store: Datastore{
                data: Arc::new(Mutex::new(decoded))
            }, 
            interval: None,
            path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_set_and_get() {
        let guard = DatastoreGuard::new(None, None).await;
        let mut datastore = guard.store();
        

        let mut list = VecDeque::new();
        list.push_back("hello".to_owned());

        let mut set = HashSet::new();
        set.insert("6.13".to_owned());

        let mut sorted = BTreeMap::new();
        sorted.insert(1, "geez".to_owned());

        let vals = vec![
            ("key1".to_owned(), Value::Text("value1".to_owned()), None),
            ("key2".to_owned(), Value::Text("value2".to_owned()), Some(Duration::from_secs(2))),
            ("key3".to_owned(), Value::List(list.clone()), None),
            ("key4".to_owned(), Value::Set(set.clone()), None),
            ("Key5".to_owned(), Value::SortedSet(sorted.clone()), None)
        ];

        for (key, value, duration) in vals {
            let _ = datastore.set(key, value, duration);
        }

        let cases = vec![
            ("key1", Some(Value::Text("value1".to_owned()))),
            ("key2", None),
            ("key3", Some(Value::List(list))),
            ("key4", Some(Value::Set(set))),
            ("Key5", Some(Value::SortedSet(sorted)))
        ];

        std::thread::sleep(Duration::from_secs(4));

        for (key, res) in cases {
            assert_eq!(datastore.get(key), res);
        }
    }

    #[tokio::test]
    async fn test_remove() {
        let guard = DatastoreGuard::new(None, None).await;
        let mut datastore = guard.store();

        let _ = datastore.set("key1".to_owned(), Value::Text("value1".to_owned()), None);

        let result = datastore.remove("key1");
        assert!(result.is_ok());

        let result = datastore.remove("non_existent_key");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_expire_and_ttl() {
        let guard = DatastoreGuard::new(None, None).await;
        let mut datastore = guard.store();

        let _ = datastore.set("key1".to_owned(), Value::Text("value1".to_owned()), Some(Duration::from_secs(2)));
        let _ = datastore.set("key2".to_owned(), Value::Text("value2".to_owned()),None);

        assert!(datastore.ttl("key1").is_ok());
        assert_eq!(datastore.ttl("key2"), Ok(None));

        std::thread::sleep(Duration::from_secs(4));

        assert!(datastore.ttl("key1").is_err());
        assert!(datastore.get("key1").is_none());
    }
    
    #[tokio::test]
    async fn test_modify_existing_key() {
        let guard = DatastoreGuard::new(None, None).await;
        let mut datastore = guard.store();
        
        let _ = datastore.set("key".to_owned(), Value::Text("value".to_owned()), None);

        let res = datastore.modify("key", |value| {
            if let Value::Text(ref mut v) = value {
                *v = "new_value".to_owned();
            }
        });

        assert!(res.is_ok());
        assert_eq!(datastore.get("key"), Some(Value::Text("new_value".to_owned())));
    }

    #[tokio::test]
    async fn test_encode_decode() {
        let guard = DatastoreGuard::new(None, None).await;
        let mut datastore = guard.store();

        let (key, value) = ("key", Value::Text("value".to_owned()));
        let (key1, value1) = ("key1", Value::Text("value1".to_owned()));

        let _ = datastore.set(key.to_owned(), value.clone(), None);
        let _ = datastore.set(key1.to_owned(), value1.clone(), None);

        let mut test_data = HashMap::new();
        test_data.insert(key.to_owned(), Item{value:value, expiry: None});
        test_data.insert(key1.to_owned(), Item{value: value1, expiry: None});

        let encoded = datastore.encode();
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

        let guard = DatastoreGuard::new(Some(backup_interval), None).await;
        let mut store = guard.store();
        let _ = store.set("data".to_owned(), Value::Text(data), None);

        guard.run_backup();

        sleep(Duration::from_secs(2)).await;

        // Assert that the backup file was created
        let backup_file_exists = std::path::Path::new("./backup/backup.efs").exists();
        assert!(backup_file_exists);
    }
}
