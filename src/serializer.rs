use bincode::{Encode, Decode};

use crate::errors::SerializerError;

pub trait bound: Encode + Decode{} 

pub fn encode<T: bound>(input: T) -> Result<Vec<u8>, SerializerError> {
    let config = bincode::config::standard();
    bincode::encode_to_vec(&input, config).map_err(|_| SerializerError::InvalidValueType)
}

pub fn decode<T: bound>(encoded: Vec<u8>) -> Result<T, SerializerError> {
    let config = bincode::config::standard();
    if let Ok(decoded) = bincode::decode_from_slice(&encoded[..], config) {
        Ok(decoded.0)
    } else {
        Err(SerializerError::InvalidValueType)
    }

}