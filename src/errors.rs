use thiserror::Error;

#[derive(Error, Debug)]
pub enum FsError {
    #[error("WriteFailed couldn't write to disk")]
    WriteFailed,
    #[error("ReadFailed couldn't read from disk")]
    ReadFailed
}

#[derive(Error, Debug, PartialEq)]
pub enum DatastoreError {
    #[error("KeyNotFound Key doesnt exists")]
    KeyNotFound,
    #[error("KeyExpired Key-value is expired")]
    KeyExpired,
    #[error("Other There is something wrong")]
    Other(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum ServiceError {
    #[error("KeyNotFound couldn't find the key")]
    KeyNotFound,
    #[error("ErrorWrite couldn't write value in data store")]
    ErrorWrite,
    #[error("ErrorPublish couldn't publish value on this channel (no listener)")]
    ErrorPublish,
    #[error("ErrorSubscribe couldn't subscribe to the channel")]
    ErrorSubscribe,
    #[error("InvalidValueType value specified to the key is not valid")]
    InvalidValueType,
    #[error("KeyExpired specified key is expired")]
    KeyExpired,
    #[error("UnknownCommand unknown command")]
    UnknownCommand,
    #[error("Internal internal error")]
    Other(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum SerializerError {
    #[error("InvalidValueType couldn't processed specified type.")]
    InvalidValueType,
}

#[derive(Error, Debug, PartialEq)]
pub enum PersistError {
    #[error("ErrorSave couldn't save data.")]
    ErrorSave,
    #[error("ErrorRead couldn't read data.")]
    ErrorRead,
    #[error("ErrorNoBackup couldn't find backup data.")]
    ErrorNoBackup,
}

#[derive(Error, Debug, PartialEq)]
pub enum ConsensusError {
    #[error("DeadNode node is dead")]
    DeadNode,
}
