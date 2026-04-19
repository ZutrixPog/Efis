use crate::{
    consensus::{LogEntry, PersistentState, Storage},
    serializer::{decode, encode},
};
use async_trait::async_trait;
use dashmap::DashMap;
use std::io::Read;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicUsize, LazyLock},
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio::{
    fs::{self, File, OpenOptions},
    time,
};

const METADATA_FILE: &str = "raft_meta.bin";
const LOGS_FILE: &str = "raft_log.bin";
const FLUSH_INTERVAL_MS: u64 = 100;

pub struct ConFileStorage {
    dir_path: PathBuf,
    log_writer: Arc<LogWriter>,
    log_cache: Arc<DashMap<usize, LogEntry>>,
    last_cached_index: AtomicUsize,
}

struct LogWriter {
    buffer: Mutex<Vec<u8>>,
    file: Mutex<File>,
    pending_bytes: AtomicUsize,
    flush_trigger: tokio::sync::Notify,
}

impl ConFileStorage {
    pub async fn new(dir_path: PathBuf) -> Arc<dyn Storage + Sync + Send> {
        fs::create_dir_all(&dir_path).await.ok();

        let logs_path = dir_path.join(LOGS_FILE);
        let log_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .append(true)
            .open(logs_path)
            .await
            .expect("Failed to open logs file");

        let log_writer = Arc::new(LogWriter {
            buffer: Mutex::new(Vec::with_capacity(1024 * 1024)), // 1MB buffer
            file: Mutex::new(log_file),
            pending_bytes: AtomicUsize::new(0),
            flush_trigger: tokio::sync::Notify::new(),
        });

        let storage = Arc::new(ConFileStorage {
            dir_path: dir_path.clone(),
            log_writer: log_writer.clone(),
            log_cache: Arc::new(DashMap::new()),
            last_cached_index: AtomicUsize::new(0),
        });

        storage.start_background_flusher().await;
        storage.warm_cache().await;

        storage
    }

    async fn warm_cache(&self) {
        let (_, logs) = self.restore().await.unwrap_or_default();
        let start_idx = logs.len().saturating_sub(1000);

        for (i, entry) in logs.into_iter().enumerate().skip(start_idx) {
            self.log_cache.insert(i, entry);
        }
        self.last_cached_index
            .store(start_idx + 1000, Ordering::Relaxed);
    }

    async fn start_background_flusher(&self) {
        let writer = self.log_writer.clone();

        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_millis(FLUSH_INTERVAL_MS));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        writer.flush().await.ok();
                    }
                    _ = writer.flush_trigger.notified() => {
                        writer.flush().await.ok();
                    }
                }
            }
        });
    }

    async fn write_metadata_optimized(&self, state: &PersistentState) -> anyhow::Result<()> {
        static LAST_METADATA: LazyLock<Mutex<Option<PersistentState>>> =
            LazyLock::new(|| Mutex::new(None));

        {
            let last = LAST_METADATA.lock().await;
            if let Some(ref last_state) = *last {
                if state.current_term == last_state.current_term
                    && state.voted_for == last_state.voted_for
                {
                    return Ok(());
                }
            }
        }

        let data = encode(state)?;

        let tmp_path = self.dir_path.join(format!("{}.tmp", METADATA_FILE));
        let meta_path = self.dir_path.join(METADATA_FILE);

        let result = tokio::task::spawn_blocking(move || {
            std::fs::write(&tmp_path, &data)?;
            std::fs::rename(&tmp_path, &meta_path)?;
            anyhow::Result::<()>::Ok(())
        })
        .await??;

        {
            let mut last = LAST_METADATA.lock().await;
            *last = Some(state.clone());
        }

        Ok(result)
    }
}

impl LogWriter {
    async fn append_batch(&self, entries: Vec<LogEntry>) -> anyhow::Result<()> {
        let total_size: usize = entries.iter().map(|e| std::mem::size_of_val(e) + 4).sum();

        if self.pending_bytes.load(Ordering::Acquire) + total_size > 1024 * 1024 {
            self.flush().await?;
        }

        let mut buffer = self.buffer.lock().await;

        for entry in entries {
            let encoded = encode(&entry)?;
            let len = encoded.len() as u32;

            buffer.extend_from_slice(&len.to_le_bytes());
            buffer.extend_from_slice(&encoded);
        }

        self.pending_bytes.fetch_add(total_size, Ordering::Release);

        if buffer.len() > 512 * 1024 {
            self.flush_trigger.notify_one();
        }

        Ok(())
    }

    async fn flush(&self) -> anyhow::Result<()> {
        let buffer = {
            let mut buf = self.buffer.lock().await;
            if buf.is_empty() {
                return Ok(());
            }
            std::mem::take(&mut *buf)
        };

        if buffer.is_empty() {
            return Ok(());
        }

        let mut file = self.file.lock().await;
        file.write_all(&buffer).await?;

        static FLUSH_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let count = FLUSH_COUNT.fetch_add(1, Ordering::Relaxed);

        if count % 10 == 0 {
            file.sync_data().await?;
        }

        self.pending_bytes.store(0, Ordering::Release);

        Ok(())
    }
}

#[async_trait]
impl Storage for ConFileStorage {
    async fn store(&self, state: PersistentState) -> anyhow::Result<()> {
        let critical_state = PersistentState {
            current_term: state.current_term,
            voted_for: state.voted_for,
            last_applied: state.last_applied,
        };

        self.write_metadata_optimized(&critical_state).await
    }

    async fn append_entry(&self, entries: Vec<LogEntry>) -> anyhow::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let start_index = self.last_cached_index.load(Ordering::Acquire);
        for (i, entry) in entries.iter().enumerate() {
            self.log_cache.insert(start_index + i, entry.clone());
        }
        self.last_cached_index
            .fetch_add(entries.len(), Ordering::Release);

        self.log_writer.append_batch(entries).await
    }

    async fn restore(&self) -> anyhow::Result<(PersistentState, Vec<LogEntry>)> {
        let cached_logs: Vec<LogEntry> = self
            .log_cache
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        if !cached_logs.is_empty() {
            let meta_path = self.dir_path.join(METADATA_FILE);
            let state = if meta_path.exists() {
                let content = fs::read(&meta_path).await?;
                decode(&content)?
            } else {
                PersistentState::default()
            };

            return Ok((state, cached_logs));
        }

        let meta_path = self.dir_path.join(METADATA_FILE);
        let state = if meta_path.exists() {
            let content = fs::read(&meta_path).await?;
            decode(&content)?
        } else {
            PersistentState::default()
        };

        let logs_path = self.dir_path.join(LOGS_FILE);
        let mut logs = Vec::new();

        if logs_path.exists() {
            let path = logs_path.clone();
            let log_result = tokio::task::spawn_blocking(move || {
                let mut logs = Vec::new();
                let file = std::fs::File::open(&path)?;
                let mut reader = std::io::BufReader::new(file);

                loop {
                    let mut len_bytes = [0u8; 4];
                    if reader.read_exact(&mut len_bytes).is_err() {
                        break;
                    }

                    let len = u32::from_le_bytes(len_bytes) as usize;
                    let mut buf = vec![0u8; len];
                    if reader.read_exact(&mut buf).is_err() {
                        break;
                    }

                    let entry: LogEntry = decode(&buf)?;
                    logs.push(entry);
                }

                anyhow::Result::<Vec<LogEntry>>::Ok(logs)
            })
            .await??;

            logs = log_result;
        }

        for (i, entry) in logs.iter().enumerate() {
            self.log_cache.insert(i, entry.clone());
        }
        self.last_cached_index.store(logs.len(), Ordering::Release);

        Ok((state, logs))
    }
}

impl Default for PersistentState {
    fn default() -> Self {
        PersistentState {
            current_term: 0,
            voted_for: None,
            last_applied: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_restore() {
        let test_state = PersistentState {
            current_term: 1,
            voted_for: Some("3".to_string()),
            last_applied: None,
        };

        let logs = vec![LogEntry {
            term: 10,
            command: crate::commands::Command::Unknown,
        }];

        let storage = ConFileStorage::new(PathBuf::from("/tmp")).await;
        assert!(!storage.store(test_state.clone()).await.is_err());
        assert!(!storage.append_entry(logs.clone()).await.is_err());

        let restored = storage.restore().await;

        let (restored_state, restored_logs) = restored.unwrap();
        assert_eq!(restored_state, test_state);
        assert_eq!(*restored_logs.last().unwrap(), logs[0]);
    }
}
