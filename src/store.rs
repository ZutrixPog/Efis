use std::collections::{HashMap, HashSet, BinaryHeap};
use std::time::{Instant, Duration};

pub type Key = String;

// TODO: refactor this
pub enum Value {
    String,
    Number,
    List,
    Set,
    SortedSet,
}

struct Item<T: Ord> {
    value: T,
    expiry: Option<Instant>,
}

enum DatastoreError {
    KeyNotFound,
    KeyExpired,
    Other(String),
}

pub trait Datastore {
    type Value;

    fn set(&mut self, key: String, value: Self::Value, expire_duration: Option<Duration>) -> Result<(), DatastoreError>;

    fn get(&self, key: &str) -> Option<&Self::Value>;
    
    fn remove(&mut self, key: &str) -> Result<Self::Value, DatastoreError>;
    
    fn expire(&mut self, key: &str, expire_duration: Duration) -> Result<(), DatastoreError>;
    
    fn ttl(&self, key: &str) -> Result<Option<Duration>, DatastoreError>;
}

pub struct MemoryDataStore {
    data: HashMap<Key, Item>,
}

impl MemoryDataStore {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }
}

impl Datastore for MemoryDataStore {
    type Value = String;

    fn set(&mut self, key: String, value: Value, expiry: Option<Duration>) {
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
        self.data.insert(key, item);
    }
    
    fn get(&self, key: &str) -> Option<&Value> {
        if let Some(item) = self.data.get(key) {
            if let Some(expiry) = item.expiry {
                if expiry <= Instant::now() {
                    self.data.remove(key);
                    return None;
                }
            }
            Some(&item.value)
        } else {
            None
        }
    }
    
    fn remove(&mut self, key: &str) -> Result<Value, DatastoreError> {
        if self.data.remove(key).is_some() {
            Ok(())
        } else {
            Err(DatastoreError::Other("couldn't delete key value"))
        }
    }
    
    fn expire(&mut self, key: &str, duration: Duration) -> Result<(), DatastoreError> {
        if let Some(item) = self.data.get_mut(key) {
            item.expiry = Some(Instant::now() + duration);
            Ok(())
        } else {
            Err(KeyNotFound)
        }
    }

    fn ttl(&self, key: &str) -> Result<Option<Duration>, DatastoreError> {
        if let Some(item) = self.data.get(key) {
            Ok(item.expiry)
        } else {
            Err(KeyNotFound)
        }
    }
    
    // fn list_add(&mut self, key: &str, value: String) -> bool {
    //     let item = self.data.entry(key.to_string()).or_insert(Item {
    //         value: Value::Set(HashSet::new()),
    //         expiry: None,
    //     });
    //     match &mut item.value {
    //         Value::Set(set) => set.insert(value),
    //         _ => false,
    //     }
    // }
    
    // fn list_all(&self, key: &str) -> Option<&HashSet<String>> {
    //     if let Some(item) = self.data.get(key) {
    //         if let Some(expiry) = item.expiry {
    //             if expiry <= Instant::now() {
    //                 self.data.remove(key);
    //                 return None;
    //             }
    //         }
    //         match &item.value {
    //             Value::Set(set) => Some(set),
    //             _ => None,
    //         }
    //     } else {
    //         None
    //     }
    // }
    
    // fn lpush(&mut self, key: &str, value: String) -> bool {
    //     let item = self.data.entry(key.to_string()).or_insert(Item {
    //         value: Value::List(Vec::new()),
    //         expiry: None,
    //     });
    //     match &mut item.value {
    //         Value::List(list) => {
    //             list.insert(0, value);
    //             true
    //         }
    //         _ => false,
    //     }
    // }
    
    // fn rpush(&mut self, key: &str, value: String) -> bool {
    //     let item = self.data.entry(key.to_string()).or_insert(Item {
    //         value: Value::List(Vec::new()),
    //         expiry: None,
    //     });
    //     match &mut item.value {
    //         Value::List(list) => {
    //             list.push(value);
    //             true
    //         }
    //         _ => false,
    //     }
    // }
    
    // fn lpop(&mut self, key: &str) -> Option<String> {
    
    // }
}
