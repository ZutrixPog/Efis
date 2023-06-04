use tokio::time::{Duration, Instant};
use std::sync::{Arc, Mutex};
use std::collections::{VecDeque, HashSet, BTreeMap, HashMap};
use std::cmp::PartialEq;
use bincode::{Encode, Decode};

use crate::errors::*;
use crate::serializer::{encode, decode};

pub type Key = String;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    List(VecDeque<String>),
    Set(HashSet<String>),
    SortedSet(BTreeMap<i64, String>),
}

#[derive(Debug, Encode, Decode)]
struct Item {
    value: Value,
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

    fn encode(&self) -> Result<Vec<u8>, DatastoreError>;

    fn from_bytes(b: Vec<u8>) -> Self;
}

#[derive(Debug)]
pub struct MemoryStoreGuard {
    store: MemoryDataStore,
}


#[derive(Debug, Clone)]
pub struct MemoryDataStore {
    data: Arc<Mutex<HashMap<String, Item>>>,
}

impl MemoryStoreGuard {
    pub fn new() -> Self {
        Self { store: MemoryDataStore::new() }
    }

    pub fn store(&self) -> MemoryDataStore {
        self.store.clone()
    }
}

impl Drop for MemoryStoreGuard {
    fn drop(&mut self) {
        self.store.shutdown_purge_task();
    }
}

impl MemoryDataStore {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    fn shutdown_purge_task(&self) {
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

    fn encode(&self) -> Result<Vec<u8>, DatastoreError> {
        // encode(self.clone()).map_err(|| DatastoreError::Other("Problem encoding data store".to_string()));
        !unimplemented!()
    }

    fn from_bytes(b: Vec<u8>) -> Self {
        !unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get() {
        let mut guard = MemoryStoreGuard::new();
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
        let mut guard = MemoryStoreGuard::new();
        let mut datastore = guard.store();

        datastore.set("key1".to_owned(), Value::Text("value1".to_owned()), None);

        let result = datastore.remove("key1");
        assert!(result.is_ok());

        let result = datastore.remove("non_existent_key");
        assert!(result.is_err());
    }

    #[test]
    fn test_expire_and_ttl() {
        let mut guard = MemoryStoreGuard::new();
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
        let mut guard = MemoryStoreGuard::new();
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
}
