use tokio::sync::broadcast;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::store::Key;

#[derive(Debug)]
pub struct PubSubGuard {
    pubsub: PubSub
}

impl PubSubGuard{
    pub fn new() -> Self {
        Self { pubsub: PubSub::new() }
    }

    pub fn ps(&self) -> PubSub {
        self.pubsub.clone()
    }
}

impl Drop for PubSubGuard {
    fn drop(&mut self) {
        self.pubsub.shutdown_purge_task();
    }
}

#[derive(Debug, Clone)]
pub struct PubSub {
    pubsub: Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>,
}

impl PubSub {
    fn new() -> Self {
        Self {
            pubsub: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn shutdown_purge_task(&self) {
        let pubsub = self.pubsub.lock().unwrap();
        drop(pubsub);
    }
}

impl PubSub {
    pub fn subscribe(&self, key: Key) -> broadcast::Receiver<String> {
        use std::collections::hash_map::Entry;

        let mut ps = self.pubsub.lock().unwrap();

        match ps.entry(key) {
            Entry::Occupied(e) => e.get().subscribe(),
            Entry::Vacant(e) => {
                let (tx, rx) = broadcast::channel(1024);
                e.insert(tx);
                rx
            }
        }
    }

    pub fn publish(&self, key: Key, value: String) -> usize {
        let ps = self.pubsub.lock().unwrap();

        ps
        .get(&key)
        .map(|tx| {tx.send(value).unwrap_or(0)})
        .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[tokio::test]
    async fn test_subscribe_existing_key() {
        let pubsub_service = PubSub::new();
        let key = String::from("test_key");
        let value = String::from("test_value");

        // Subscribe to an existing key
        let mut receiver = pubsub_service.subscribe(key.clone());

        // Publish a value to the key
        let result = pubsub_service.publish(key, value.clone());

        // Check that the value is received by the subscriber
        assert_eq!(receiver.recv().await.unwrap(), value);

        // Check that the publish operation returns 1 (one receiver)
        assert_eq!(result, 1);
    }

    #[tokio::test]
    async fn test_subscribe_new_key() {
        let pubsub_service = PubSub::new();
        let key = String::from("test_key");
        let value = String::from("test_value");

        // Subscribe to a new key
        let mut receiver = pubsub_service.subscribe(key.clone());

        // Publish a value to the key
        let result = pubsub_service.publish(key, value.clone());

        // Check that the value is received by the subscriber
        assert_eq!(receiver.recv().await.unwrap(), value);

        // Check that the publish operation returns 1 (one receiver)
        assert_eq!(result, 1);
    }

    #[tokio::test]
    async fn test_publish_nonexistent_key() {
        let pubsub_service = PubSub::new();
        let key = String::from("test_key");
        let value = String::from("test_value");

        // Publish a value to a non-existent key
        let result = pubsub_service.publish(key, value);

        // Check that the publish operation returns 0 (no receivers)
        assert_eq!(result, 0);
    }

    #[tokio::test]
    async fn test_shutdown_purge_task() {
        let pubsub_service = Arc::new(PubSub::new());

        // Spawn a thread to simulate the purge task
        let pubsub_service_clone = Arc::clone(&pubsub_service);
        let handle = thread::spawn(move || {
            pubsub_service_clone.shutdown_purge_task();
        });

        // Ensure that the thread completes without errors
        handle.join().unwrap();
    }
}