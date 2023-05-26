use std::time::{Instant, Duration};
use std::sync::{Arc, Mutex};
use std::collections::{VecDeque, HashSet, BTreeMap, HashMap};
use std::cmp::PartialEq;
use thiserror::Error; // 1.0.40

pub type Key = String;

#[derive(Error, Debug, PartialEq)]
pub enum DatastoreError {
    #[error("Key doesnt exists")]
    KeyNotFound,
    #[error("Key-value is expired")]
    KeyExpired,
    #[error("There is something wrong")]
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    List(VecDeque<String>),
    Set(HashSet<String>),
    SortedSet(BTreeMap<i64, String>),
}

struct Item {
    value: Arc<Mutex<Value>>,
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

pub struct MemoryDataStore {
    data: Arc<Mutex<HashMap<String, Item>>>,
}

impl MemoryDataStore {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Datastore for MemoryDataStore {
    type Type = Value;    

    fn set(&mut self, key: String, value: Self::Type, expiry: Option<Duration>) -> Result<(), DatastoreError> {
        let item = Item {
            value: Arc::new(Mutex::new(value)),
            expiry: expiry.map(|d| Instant::now() + d),
        };
        let mut data = self.data.lock().unwrap();
        data.insert(key, item);
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
            
            Some((*item.value.lock().unwrap()).clone())
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
            let value = Arc::get_mut(&mut item.value).ok_or(DatastoreError::Other("Concurrent access error".to_owned()))?;
            modifier(value.get_mut().unwrap());
            Ok(())
        } else {
            Err(DatastoreError::KeyNotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get() {
        let mut datastore = MemoryDataStore::new();

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
        let mut data_store = MemoryDataStore::new();

        data_store.set("key1".to_owned(), Value::Text("value1".to_owned()), None);

        let result = data_store.remove("key1");
        assert!(result.is_ok());

        let result = data_store.remove("non_existent_key");
        assert!(result.is_err());
    }

    #[test]
    fn test_expire_and_ttl() {
        let mut data_store = MemoryDataStore::new();

        data_store.set("key1".to_owned(), Value::Text("value1".to_owned()), Some(Duration::from_secs(2)));
        data_store.set("key2".to_owned(), Value::Text("value2".to_owned()),None);

        assert!(data_store.ttl("key1").is_ok());
        assert_eq!(data_store.ttl("key2"), Ok(None));

        std::thread::sleep(Duration::from_secs(4));

        assert!(data_store.ttl("key1").is_err());
        assert!(data_store.get("key1").is_none());
    }
    
    
    #[test]
    fn test_modify_existing_key() {
        let mut datastore = MemoryDataStore::new();
        datastore.set("key".to_owned(), Value::Text("value".to_owned()), None);

        // Modify the value of an existing key
        let res = datastore.modify("key", |value| {
            if let Value::Text(ref mut v) = value {
                *v = "new_value".to_owned();
            }
        });

        // Verify that the value has been modified
        assert!(res.is_ok());
        assert_eq!(datastore.get("key"), Some(Value::Text("new_value".to_owned())));
    }
}
