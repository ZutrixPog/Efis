use serde::{Deserialize, Serialize};

use crate::errors::SerializerError;

pub fn encode<T: Serialize>(input: T) -> Result<Vec<u8>, SerializerError> {
    bincode::serialize(&input).map_err(|_| SerializerError::InvalidValueType)
}

pub fn decode<'a, T: Deserialize<'a>>(encoded: &'a Vec<u8>) -> Result<T, SerializerError> {
    if let Ok(decoded) = bincode::deserialize::<T>(&encoded[..]) {
        Ok(decoded)
    } else {
        Err(SerializerError::InvalidValueType)
    }

}