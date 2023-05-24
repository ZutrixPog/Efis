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
    value: Value,
    expiry: Option<Instant>,
}

pub trait Datastore {
    type Type;

    fn set(&mut self, key: String, value: Self::Type, expire_duration: Option<Duration>) -> Result<(), DatastoreError>;

    fn get(&self, key: &str) -> Option<&Self::Type>;
    
    fn remove(&mut self, key: &str) -> Result<Self::Type, DatastoreError>;
    
    fn expire(&mut self, key: &str, expire_duration: Duration) -> Result<(), DatastoreError>;
    
    fn ttl(&self, key: &str) -> Result<Option<Duration>, DatastoreError>;
}

pub struct MemoryDataStore {
    data: Arc<Mutex<HashMap<Key, Item>>>,
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
        let item = match expiry {
            Some(duration) => Item {
                value,
                expiry: Some(Instant::now() + duration),
            },
            None => Item {
                value,
                expiry: None,
            },
        };

        let mut data = self.data.lock().unwrap();
        data.insert(key, item);
        Ok(())
    }

    fn get(&self, key: &str) -> Option<&Self::Type> {
        let mut data = self.data.lock().unwrap();
        if let Ok(Some(expiry)) = self.ttl(key) {
            if expiry.is_zero() {
                data.remove(key);
                return None;
            }
            Some(&(data.get(key).unwrap().value.clone()))
        } else {
            None
        }
    }

    fn remove(&mut self, key: &str) -> Result<Self::Type, DatastoreError> {
        let mut data = self.data.lock().unwrap();
        if let Some(item) = data.remove(key) {
            Ok(item.value)
        } else {
            Err(DatastoreError::Other("couldn't delete key value".to_string()))
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
        let data = self.data.lock().unwrap();
        if let Some(item) = data.get(key) {
            if let Some(exp) = item.expiry {
                Ok(Some(exp - Instant::now()))
            } else {
                Ok(None)
            }
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
