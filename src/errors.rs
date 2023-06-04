use std::error;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FsError {
    #[error("couldn't write to disk")]
    WriteFailed,
    #[error("couldn't read from disk")]
    ReadFailed
}

#[derive(Error, Debug, PartialEq)]
pub enum DatastoreError {
    #[error("Key doesnt exists")]
    KeyNotFound,
    #[error("Key-value is expired")]
    KeyExpired,
    #[error("There is something wrong")]
    Other(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum ServiceError {
    #[error("Service: couldn't find the key")]
    KeyNotFound,
    #[error("Service: couldn't write value in data store.")]
    ErrorWrite,
    #[error("Service: value specified to the key is not valid.")]
    InvalidValueType,
    #[error("Service: specified key is expired.")]
    KeyExpired,
    #[error("Service: couldn't decrement value.")]
    Other(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum SerializerError {
    #[error("couldn't processed specified type.")]
    InvalidValueType,
}