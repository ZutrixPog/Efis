use std::time::{Instant, Duration};

pub type Key = String;

// TODO: It's better wrap theses but since they are std data structures, They won't cause us any trouble.
pub enum Value {
    Text(String),
    Number(f64),
    List(VecDeque<Value>),
    Set(HashSet<Value>),
    SortedSet(BTreeSet<Value>),
}

struct Item<T> {
    value: T,
    expiry: Option<Instant>,
}

pub trait Datastore {
    type Type;

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
    type Type = Value;

    fn set(&mut self, key: String, value: Type, expiry: Option<Duration>) {
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
    
    fn get(&self, key: &str) -> Option<&Type> {
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
    
    fn remove(&mut self, key: &str) -> Result<Type, DatastoreError> {
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
}
