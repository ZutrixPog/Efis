use std::clone;
use std::path::{PathBuf, Path};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs::{self, File, OpenOptions};
use rand::Rng;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::errors::PersistError;

const FILE_NAME: &str = "backup.efs";

#[derive(Clone, Debug)]
pub struct FileBackupRepo {
    path: PathBuf,
}

impl FileBackupRepo {
    pub fn new(path: PathBuf) -> Self {
        fs::create_dir_all(path.clone());
        Self {
            path
        }
    }

    pub async fn save(&self, data: Vec<u8>) -> Result<(), PersistError> {
        let path = self.path.join(FILE_NAME);
        let tmp_path =  self.path.join(generate_tmp_name());
        let tmp = Path::new(&tmp_path);
        let mut fp = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(tmp).await
            .map_err(|_| PersistError::ErrorSave)?;

        if let Err(_) = fp.write_all(&data).await {
            fs::remove_file(tmp).await.map_err(|_| PersistError::ErrorSave)?;
        }
        
        if let Err(_) = fp.sync_all().await {
            fs::remove_file(tmp).await.map_err(|_| PersistError::ErrorSave)?;
        }

        fs::rename(tmp_path, Path::new(&path)).await.map_err(|_| PersistError::ErrorSave)
    }
    
    pub async fn retrieve(&self) -> Result<Vec<u8>, PersistError> {
        // let mut latest_file: Option<PathBuf> = None;
        // let mut latest_modification_time = SystemTime::UNIX_EPOCH;

        // let mut entries = fs::read_dir(&self.path).await.map_err(|_| PersistError::ErrorRead)?;
        // while let Some(entry) = entries.next_entry().await.map_err(|_| PersistError::ErrorRead)? {
        //     if let Ok(metadata) = entry.metadata().await {
        //         let modified_time = metadata.created().unwrap_or(UNIX_EPOCH);

        //         if modified_time > latest_modification_time {
        //             latest_file = Some(entry.path());
        //             latest_modification_time = modified_time;
        //         }
        //     }
        // }
        
        let file_path = self.path.join(FILE_NAME);
        let mut file = File::open(&file_path).await.map_err(|_| PersistError::ErrorNoBackup)?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).await.map_err(|_| PersistError::ErrorRead)?;
        Ok(contents)
    }
}

fn generate_tmp_name() -> String {
    let mut rng = rand::thread_rng();

    format!("tmp.{}", rng.gen::<i32>())
}

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
    assert_eq!(retrieve_result.unwrap_err(), PersistError::ErrorNoBackup);
}