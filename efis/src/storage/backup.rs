use rand::Rng;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FILE_NAME: &str = "backup.efs";

#[derive(Clone, Debug)]
pub struct FileBackupRepo {
    path: PathBuf,
}

impl FileBackupRepo {
    pub fn new(path: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(path.clone());
        Self { path }
    }

    pub async fn save(&self, data: Vec<u8>) -> anyhow::Result<()> {
        let err = anyhow::format_err!("failed to persist");
        let path = self.path.join(FILE_NAME);
        let tmp_path = self.path.join(generate_tmp_name());
        let tmp = Path::new(&tmp_path);
        let mut fp = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(tmp)
            .await
            .map_err(|_| anyhow::format_err!("failed to persist"))?;

        if let Err(_) = fp.write_all(&data).await {
            fs::remove_file(tmp)
                .await
                .map_err(|_| anyhow::format_err!("failed to persist"))?;
            return Err(anyhow::format_err!("failed to persist"));
        }

        if let Err(_) = fp.sync_all().await {
            fs::remove_file(tmp)
                .await
                .map_err(|_| anyhow::format_err!("failed to persist"))?;
            return Err(anyhow::format_err!("failed to persist"));
        }

        fs::rename(tmp, Path::new(&path)).await.map_err(|_| err)
    }

    pub async fn retrieve(&self) -> anyhow::Result<Vec<u8>> {
        let file_path = self.path.join(FILE_NAME);
        let mut file = File::open(&file_path)
            .await
            .map_err(|_| anyhow::format_err!("backup file not found"))?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .await
            .map_err(|_| anyhow::format_err!("failed to read backup file"))?;
        Ok(contents)
    }
}

fn generate_tmp_name() -> String {
    let mut rng = rand::thread_rng();

    format!("tmp.{}", rng.gen::<i32>())
}

#[cfg(test)]
mod tests {
    use crate::storage::backup::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_file_backup_repo_save_and_retrieve() {
        let temp_dir = tempdir().unwrap();
        let repo = FileBackupRepo::new(temp_dir.path().to_path_buf());

        // Prepare test data
        let data = b"test data".to_vec();

        // Save the data
        let save_result = repo.save(data.clone()).await;
        assert!(save_result.is_ok());

        // Retrieve the data
        let retrieve_result = repo.retrieve().await;
        assert!(retrieve_result.is_ok());
        let retrieved_data = retrieve_result.unwrap();
        assert_eq!(retrieved_data, data);
    }

    #[tokio::test]
    async fn test_file_backup_repo_retrieve_no_backup() {
        let temp_dir = tempdir().unwrap();
        let repo = FileBackupRepo::new(temp_dir.path().to_path_buf());

        let retrieve_result = repo.retrieve().await;
        assert!(retrieve_result.is_err());
    }
}
