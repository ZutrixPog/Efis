use std::error;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FsError {
    #[error("couldn't write to disk")]
    WriteFailed,
    #[error("couldn't read from disk")]
    ReadFailed
}

#[derive(Error, Debug)]
pub enum DatastoreError {
    #[error("Key doesnt exists")]
    KeyNotFound,
    #[error("Key-value is expired")]
    KeyExpired,
    #[error("There is something wrong")]
    Other(String),
}