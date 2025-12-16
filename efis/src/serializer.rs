use serde::{Deserialize, Serialize};

pub fn encode<T: Serialize>(input: T) -> anyhow::Result<Vec<u8>> {
    bincode::serialize(&input).map_err(|_| anyhow::format_err!("invalid type"))
}

pub fn decode<'a, T: Deserialize<'a>>(encoded: &'a Vec<u8>) -> anyhow::Result<T> {
    if let Ok(decoded) = bincode::deserialize::<T>(&encoded[..]) {
        Ok(decoded)
    } else {
        Err(anyhow::format_err!("invalid type"))
    }
}
