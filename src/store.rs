use tokio::time::{Duration, Instant, interval, sleep};
use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::{VecDeque, HashSet, BTreeMap, HashMap};
use std::cmp::PartialEq;
use std::path::{PathBuf, Path};
use serde::{Deserialize, Serialize};

use crate::errors::*;
use crate::persist::repo::FileBackupRepo;
use crate::serializer::{encode, decode};


const path: &str = "./backup";
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

pub trait Datastore {
    type Type;
    
    fn set(&mut self, key: String, value: Self::Type, expire_duration: Option<Duration>) -> Result<(), DatastoreError>;

    fn get(&self, key: &str) -> Option<Self::Type>;
    
    fn modify<F>(&mut self, key: &str, modifier: F) -> Result<(), DatastoreError> where F: FnOnce(&mut Self::Type);
    
    fn remove(&mut self, key: &str) -> Result<(), DatastoreError>;
    
    fn expire(&mut self, key: &str, expire_duration: Duration) -> Result<(), DatastoreError>;
    
    fn ttl(&self, key: &str) -> Result<Option<Duration>, DatastoreError>;
}

#[derive(Debug)]
pub struct MemoryStoreGuard {
    store: MemoryDataStore,
    interval: Option<Duration>
}

impl MemoryStoreGuard {
    pub fn new(interval: Option<Duration>) -> Self {
        Self { store: MemoryDataStore::new(interval), interval }
    }
    
    pub fn store(&self) -> MemoryDataStore {
        self.store.clone()
    }

    pub fn run_backup(&self) {
        if self.interval.is_none() {
            return;
        }

        let dur = self.interval.unwrap().clone();
        let data = self.store();
        tokio::spawn(async move {
            let repo = FileBackupRepo::new(Path::new(path).to_path_buf());
            
            loop {
                thread::sleep(dur);
                
                let data = data.encode().unwrap();
                if let Err(_) = repo.save(data).await {
                    return;
                }
            }
        });
    }
}

impl Drop for MemoryStoreGuard {
    fn drop(&mut self) {
        self.store.shutdown_purge_task();
    }
}

#[derive(Debug, Clone)]
pub struct MemoryDataStore {
    data: Arc<Mutex<HashMap<String, Item>>>,
}

impl MemoryDataStore {
    pub fn new(interval: Option<Duration>) -> Self {
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

impl Datastore for MemoryDataStore {
    type Type = Value;    

    fn set(&mut self, key: String, value: Self::Type, expiry: Option<Duration>) -> Result<(), DatastoreError> {
        let item = Item {
            value: value,
            expiry: expiry.map(|d| Instant::now() + d),
        };
        let mut data = self.data.lock().unwrap();
        data.entry(key).or_insert(item);
        Ok(())
    }

    fn get(&self, key: &str) -> Option<Self::Type> {
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

    fn remove(&mut self, key: &str) -> Result<(), DatastoreError> {
        let mut data = self.data.lock().unwrap();
        if data.remove(key).is_some() {
            Ok(())
        } else {
            Err(DatastoreError::KeyNotFound)
        }
    }

    fn expire(&mut self, key: &str, duration: Duration) -> Result<(), DatastoreError> {
        let mut data = self.data.lock().unwrap();
        if let Some(item) = data.get_mut(key) {
            item.expiry = Some(Instant::now() + duration);
            Ok(())
        } else {
            Err(DatastoreError::KeyNotFound)
        }
    }

    fn ttl(&self, key: &str) -> Result<Option<Duration>, DatastoreError> {
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

    fn modify<F>(&mut self, key: &str, modifier: F) -> Result<(), DatastoreError>
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

impl TryFrom<Vec<u8>> for MemoryStoreGuard {
    type Error = DatastoreError;
    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let decoded = decode(&value)
            .map_err(|_| DatastoreError::Other("problem decoding".to_string()))?;

        Ok(MemoryStoreGuard {
            store: MemoryDataStore{
                data: Arc::new(Mutex::new(decoded))
            }, 
            interval: None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get() {
        let mut guard = MemoryStoreGuard::new(None);
        let mut datastore = guard.store();
        

        let mut list = VecDeque::new();
        list.push_back("hello".to_owned());

        let mut set = HashSet::new();
        set.insert("6.13".to_owned());

        let mut sorted = BTreeMap::new();
        sorted.insert(1, "geez".to_owned());

        let mut vals = vec![
            ("key1".to_owned(), Value::Text("value1".to_owned()), None),
            ("key2".to_owned(), Value::Text("value2".to_owned()), Some(Duration::from_secs(2))),
            ("key3".to_owned(), Value::List(list.clone()), None),
            ("key4".to_owned(), Value::Set(set.clone()), None),
            ("Key5".to_owned(), Value::SortedSet(sorted.clone()), None)
        ];

        for (key, value, duration) in vals {
            datastore.set(key, value, duration);
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

    #[test]
    fn test_remove() {
        let mut guard = MemoryStoreGuard::new(None);
        let mut datastore = guard.store();

        datastore.set("key1".to_owned(), Value::Text("value1".to_owned()), None);

        let result = datastore.remove("key1");
        assert!(result.is_ok());

        let result = datastore.remove("non_existent_key");
        assert!(result.is_err());
    }

    #[test]
    fn test_expire_and_ttl() {
        let mut guard = MemoryStoreGuard::new(None);
        let mut datastore = guard.store();

        datastore.set("key1".to_owned(), Value::Text("value1".to_owned()), Some(Duration::from_secs(2)));
        datastore.set("key2".to_owned(), Value::Text("value2".to_owned()),None);

        assert!(datastore.ttl("key1").is_ok());
        assert_eq!(datastore.ttl("key2"), Ok(None));

        std::thread::sleep(Duration::from_secs(4));

        assert!(datastore.ttl("key1").is_err());
        assert!(datastore.get("key1").is_none());
    }
    
    #[test]
    fn test_modify_existing_key() {
        let mut guard = MemoryStoreGuard::new(None);
        let mut datastore = guard.store();
        
        datastore.set("key".to_owned(), Value::Text("value".to_owned()), None);

        let res = datastore.modify("key", |value| {
            if let Value::Text(ref mut v) = value {
                *v = "new_value".to_owned();
            }
        });

        assert!(res.is_ok());
        assert_eq!(datastore.get("key"), Some(Value::Text("new_value".to_owned())));
    }

    #[test]
    fn test_encode_decode() {
        let mut guard = MemoryStoreGuard::new(None);
        let mut datastore = guard.store();

        let (key, value) = ("key", Value::Text("value".to_owned()));
        let (key1, value1) = ("key1", Value::Text("value1".to_owned()));

        datastore.set(key.to_owned(), value.clone(), None);
        datastore.set(key1.to_owned(), value1.clone(), None);

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
        let backup_interval = Duration::from_secs(5); // Backup interval of 5 seconds
        let data = vec![1, 2, 3, 4, 5]; // Example data to be backed up

        // Create an instance of your struct, assuming it's called `BackupManager`
        let guard = MemoryStoreGuard::new(Some(backup_interval));
        // let data_store = store

        // Spawn the backup task
        guard.run_backup();

        // Sleep for a while to allow the backup task to run
        sleep(Duration::from_secs(10)).await;

        // Perform assertions or checks here to verify that the backup task executed as expected.
        // For example, you could check if the backup file was created or if the data was saved.

        // Assert that the backup file was created
        let backup_file_exists = std::path::Path::new("./backup/backup.efs").exists();
        assert!(backup_file_exists);
    }
}
