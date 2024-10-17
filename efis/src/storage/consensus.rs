use crate::{
    consensus::{PersistentState, Storage},
    errors::PersistError,
    serializer::{decode, encode},
};
use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error, info, warn};

const FILE_NAME: &str = "cons";

pub struct ConFileStorage {
    path: PathBuf,
}

impl ConFileStorage {
    pub fn new(filepath: PathBuf) -> Arc<dyn Storage + Sync + Send> {
        Arc::new(ConFileStorage { path: filepath })
    }
}

#[async_trait]
impl Storage for ConFileStorage {
    async fn store(&self, state: PersistentState) -> anyhow::Result<()> {
        let path = self.path.join(FILE_NAME);
        let mut fp = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .await
            .map_err(|err| {
                error!("{}", err.to_string());
                PersistError::ErrorSave
            })?;

        let data = encode(state)?;

        fp.write_all(&data).await?;
        fp.sync_all().await?;

        Ok(())
    }

    async fn restore(&self) -> anyhow::Result<PersistentState> {
        let file_path = self.path.join(FILE_NAME);
        let mut file = File::open(&file_path).await?;

        let mut contents = Vec::new();
        file.read_to_end(&mut contents).await?;

        let state = decode(&contents)?;

        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_restore() {
        let test_state = PersistentState {
            current_term: 1,
            voted_for: 3,
            logs: vec![],
        };

        let storage = ConFileStorage::new(PathBuf::from("./"));
        assert!(!storage.store(test_state.clone()).await.is_err());

        let restored = storage.restore().await;

        assert_eq!(restored.unwrap(), test_state);
    }
}
