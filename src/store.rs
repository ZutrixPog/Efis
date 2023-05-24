use std::time::{Instant, Duration};
use std::sync::{Arc, Mutex};
use std::collections::{VecDeque, HashSet, BTreeSet, HashMap};
use thiserror::Error; // 1.0.40

pub type Key = String;

#[derive(Error, Debug)]
pub enum DatastoreError {
    #[error("Key doesnt exists")]
    KeyNotFound,
    #[error("Key-value is expired")]
    KeyExpired,
    #[error("There is something wrong")]
    Other(String),
}

// TODO: It's better wrap theses but since they are std data structures, They won't cause us any trouble.
#[derive(Debug)]
pub enum Value {
    Text(String),
    Number(f64),
    List(VecDeque<Value>),
    Set(HashSet<Value>),
    SortedSet(BTreeSet<Value>),
}

struct Item {
    value: Arc<Mutex<Value>>,
    expiry: Option<Instant>,
}

pub trait Datastore {
    type Type;

    fn set(&mut self, key: String, value: Self::Type, expire_duration: Option<Duration>) -> Result<(), DatastoreError>;

    fn get(&self, key: &str) -> Option<&Self::Type>;

    fn modify<F>(&mut self, key: &str, modifier: F) -> Result<(), String> where F: FnOnce(&mut Value);
    
    fn remove(&mut self, key: &str) -> Result<Self::Type, DatastoreError>;
    
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

    pub fn set(&mut self, key: String, value: Value, expiry: Option<Duration>) {
        let item = Item {
            value: Arc::new(Mutex::new(value)),
            expiry: expiry.map(|d| Instant::now() + d),
        };
        let mut data = self.data.lock().unwrap();
        data.insert(key, item);
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let data = self.data.lock().unwrap();
        if let Some(item) = data.get(key) {
            if let Some(expiry) = item.expiry {
                if expiry <= Instant::now() {
                    data.remove(key);
                    return None;
                }
            }
            Some(item.value.lock().unwrap().clone())
        } else {
            None
        }
    }

    pub fn remove(&mut self, key: &str) -> Result<(), String> {
        let mut data = self.data.lock().unwrap();
        if data.remove(key).is_some() {
            Ok(())
        } else {
            Err("Key not found".to_string())
        }
    }

    pub fn expire(&mut self, key: &str, duration: Duration) -> Result<(), String> {
        let mut data = self.data.lock().unwrap();
        if let Some(item) = data.get_mut(key) {
            item.expiry = Some(Instant::now() + duration);
            Ok(())
        } else {
            Err("Key not found".to_string())
        }
    }

    pub fn ttl(&self, key: &str) -> Result<Option<Duration>, String> {
        let data = self.data.lock().unwrap();
        if let Some(item) = data.get(key) {
            if let Some(expiry) = item.expiry {
                let now = Instant::now();
                if now >= expiry {
                    data.remove(key);
                    return Ok(None);
                }
                Ok(Some(expiry - now))
            } else {
                Ok(None)
            }
        } else {
            Err("Key not found".to_string())
        }
    }

    pub fn modify<F>(&mut self, key: &str, modifier: F) -> Result<(), String>
    where
        F: FnOnce(&mut Value),
    {
        let mut data = self.data.lock().unwrap();
        if let Some(item) = data.get_mut(key) {
            let value = Arc::get_mut(&mut item.value).ok_or("Concurrent access error")?;
            modifier(value);
            Ok(())
        } else {
            Err("Key not found".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get() {
        let mut data_store = MemoryDataStore::new();

        data_store.set("key1".to_owned(), Value::Text("value1".to_owned()), None);
        data_store.set("key2".to_owned(), Value::Text("value2".to_owned()), Some(Duration::from_secs(10)));
        data_store.set("key3".to_owned(), Value::Number(34.3), None);
        data_store.set("key4".to_owned(), Value::List(VecDeque::new()), None);
        data_store.set("key5".to_owned(), Value::Set(HashSet::new()), Some(Duration::from_secs(43)));
        data_store.set("Key6".to_owned(), Value::SortedSet(BTreeSet::new()), None);

        assert_eq!(data_store.get("key1"), Some(&Value::Text("value1".to_owned())));
        assert_eq!(data_store.get("key2"), Some(&Value::Text("value2".to_owned())));
        assert_eq!(data_store.get("non_existent_key"), None);
        assert_eq!(data_store.get("key3"), Some(&Value::Number(34.3)));
        assert_eq!(data_store.get("key4"), Some(&Value::List(VecDeque::new())));
        // assert_eq!(data_store.get("key5"), Some(&Value::Text("value2".to_owned())));
        // assert_eq!(data_store.get("key6"), Some(&Value::Text("value2".to_owned())));
    }

    #[test]
    fn test_remove() {
        let mut data_store = MemoryDataStore::new();

        data_store.set("key1".to_owned(), "value1".to_owned(), None);
        data_store.set("key2".to_owned(), "value2".to_owned(), None);

        let result = data_store.remove("key1");
        assert!(result.is_ok());

        let result = data_store.remove("non_existent_key");
        assert!(result.is_err());
    }

    #[test]
    fn test_expire_and_ttl() {
        let mut data_store = MemoryDataStore::new();

        data_store.set("key1".to_owned(), "value1".to_owned(), Some(Duration::from_secs(1)));
        data_store.set("key2".to_owned(), "value2".to_owned(), None);

        assert_eq!(data_store.ttl("key1"), Ok(Some(Duration::from_secs(1))));
        assert_eq!(data_store.ttl("key2"), Ok(None));

        std::thread::sleep(Duration::from_secs(2));

        assert_eq!(data_store.ttl("key1"), Err(DatastoreError::KeyNotFound));
        assert_eq!(data_store.get("key1"), None);
    }
}
