use std::sync::Arc;
use thiserror::Error;

pub mod client;
pub mod dispatcher;
pub mod server;

pub trait Serialize: Sync + Send {
    fn serialize(&self) -> String;
}

pub trait Deserialize: Sized + Sync + Send {
    fn deserialize(s: &str) -> Result<Self, String>;
}

pub trait SerDe: Serialize + Deserialize + Clone + Send + Sync {
    fn as_any(&self) -> Arc<dyn std::any::Any>;
}

impl<T> SerDe for T
where
    T: Serialize + Deserialize + Clone + Send + Sync + 'static,
{
    fn as_any(&self) -> Arc<dyn std::any::Any> {
        Arc::new(self.clone())
    }
}

#[derive(Error, Debug)]
pub enum RpcError {
    #[error("method not found: {0}")]
    MethodNotFound(String),

    #[error("failed to deserialize: {0}")]
    Deserialize(String),

    #[error("handler failed: {0}")]
    Internal(#[from] anyhow::Error),

    #[error("not leader, redirect to {0}")]
    NotLeader(String),
}

impl Serialize for RpcError {
    fn serialize(&self) -> String {
        match self {
            RpcError::NotLeader(id) => format!("{{error=\"not_leader\" leader={}}}", id),
            _ => format!("{{error=\"{}\"}}", self),
        }
    }
}

#[derive(macros::SerDe)]
pub struct ErrorRes {
    error: String,
}

pub trait RpcStruct {
    fn register_fns(&'static self, dispatcher: &mut dispatcher::Dispatcher);
}

impl Serialize for i32 {
    fn serialize(&self) -> String {
        self.to_string()
    }
}
impl Deserialize for i32 {
    fn deserialize(s: &str) -> Result<Self, String> {
        s.parse().map_err(|e| format!("Failed to parse i32: {}", e))
    }
}

impl Serialize for i64 {
    fn serialize(&self) -> String {
        self.to_string()
    }
}
impl Deserialize for i64 {
    fn deserialize(s: &str) -> Result<Self, String> {
        s.parse().map_err(|e| format!("Failed to parse i64: {}", e))
    }
}

impl Serialize for f64 {
    fn serialize(&self) -> String {
        self.to_string()
    }
}
impl Deserialize for f64 {
    fn deserialize(s: &str) -> Result<Self, String> {
        s.parse().map_err(|e| format!("Failed to parse f64: {}", e))
    }
}

impl Serialize for f32 {
    fn serialize(&self) -> String {
        self.to_string()
    }
}
impl Deserialize for f32 {
    fn deserialize(s: &str) -> Result<Self, String> {
        s.parse().map_err(|e| format!("Failed to parse f32: {}", e))
    }
}

impl Serialize for usize {
    fn serialize(&self) -> String {
        self.to_string()
    }
}
impl Deserialize for usize {
    fn deserialize(s: &str) -> Result<Self, String> {
        s.parse()
            .map_err(|e| format!("Faield to parse usize: {} {}", e, s))
    }
}

impl Serialize for u64 {
    fn serialize(&self) -> String {
        self.to_string()
    }
}
impl Deserialize for u64 {
    fn deserialize(s: &str) -> Result<Self, String> {
        s.parse().map_err(|e| format!("Faield to parse u64: {}", e))
    }
}

impl Serialize for bool {
    fn serialize(&self) -> String {
        self.to_string()
    }
}
impl Deserialize for bool {
    fn deserialize(s: &str) -> Result<Self, String> {
        s.parse()
            .map_err(|e| format!("Faield to parse bool: {}", e))
    }
}

impl Serialize for String {
    fn serialize(&self) -> String {
        if self.contains(' ') {
            format!("\"{}\"", self.replace('"', "\\\""))
        } else {
            self.clone()
        }
    }
}
impl Deserialize for String {
    fn deserialize(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.starts_with('"') && s.ends_with('"') {
            Ok(s[1..s.len() - 1].replace("\\\"", "\""))
        } else {
            Ok(s.to_string())
        }
    }
}

impl<T: Serialize + Clone> Serialize for Option<T> {
    fn serialize(&self) -> String {
        if self.is_none() {
            return "None".to_string();
        }
        self.clone().unwrap().serialize()
    }
}
impl<T: Deserialize> Deserialize for Option<T> {
    fn deserialize(s: &str) -> Result<Self, String> {
        if s.contains("None") || s.is_empty() {
            return Ok(None);
        }
        Ok(Some(T::deserialize(s).unwrap()))
    }
}

impl<T: Serialize> Serialize for Vec<T> {
    fn serialize(&self) -> String {
        let parts: Vec<String> = self.iter().map(|v| v.serialize()).collect();
        format!("[{}]", parts.join(","))
    }
}

impl<T: Deserialize> Deserialize for Vec<T> {
    fn deserialize(s: &str) -> Result<Self, String> {
        let inner = s.trim().trim_start_matches('[').trim_end_matches(']');
        if inner.is_empty() {
            return Ok(Vec::new());
        }

        let mut result = Vec::new();
        let mut current = String::new();
        let mut depth = 0;
        let mut in_quotes = false;

        for c in inner.chars() {
            match c {
                '"' => {
                    in_quotes = !in_quotes;
                    current.push(c);
                }
                '[' | '{' if !in_quotes => {
                    depth += 1;
                    current.push(c);
                }
                ']' | '}' if !in_quotes => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 && !in_quotes => {
                    result.push(current.trim().to_string());
                    current.clear();
                }
                _ => current.push(c),
            }
        }

        if !current.trim().is_empty() {
            result.push(current.trim().to_string());
        }

        result
            .into_iter()
            .map(|x| T::deserialize(&x))
            .collect::<Result<Vec<_>, _>>()
    }
}

pub fn parse_key_values(s: &str) -> Result<std::collections::HashMap<String, String>, String> {
    use std::collections::HashMap;
    let mut map = HashMap::new();

    let mut key = String::new();
    let mut value = String::new();
    let mut in_key = true;
    let mut in_quotes = false;
    let mut depth_square = 0;
    let mut depth_curly = 0;

    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '=' if in_key => {
                in_key = false;
            }
            '"' => {
                in_quotes = !in_quotes;
                value.push(c);
            }
            '[' if !in_quotes => {
                depth_square += 1;
                value.push(c);
            }
            ']' if !in_quotes => {
                depth_square -= 1;
                value.push(c);
            }
            '{' if !in_quotes => {
                depth_curly += 1;
                value.push(c);
            }
            '}' if !in_quotes => {
                depth_curly -= 1;
                value.push(c);
            }
            ' ' if !in_quotes && depth_square == 0 && depth_curly == 0 => {
                if !key.is_empty() {
                    map.insert(key.trim().to_string(), value.trim().to_string());
                    key.clear();
                    value.clear();
                    in_key = true;
                }
            }
            _ => {
                if in_key {
                    key.push(c);
                } else {
                    value.push(c);
                }
            }
        }
    }

    if !key.is_empty() {
        map.insert(key.trim().to_string(), value.trim().to_string());
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use macros::SerDe;

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Inner {
        a: usize,
        b: bool,
    }

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Test {
        a: i32,
        b: f64,
        c: String,
        d: Vec<Inner>,
        e: Inner,
        f: Option<usize>,
    }

    #[test]
    fn test_macro() {
        let test_struct = Test {
            a: 2,
            b: 0.2,
            c: String::from("hey"),
            d: vec![Inner { a: 1, b: true }, Inner { a: 2, b: false }],
            e: Inner { a: 1, b: true },
            f: None,
        };

        let serialized = test_struct.serialize();
        assert_eq!(
            serialized,
            "{a=2 b=0.2 c=hey d=[{a=1 b=true},{a=2 b=false}] e={a=1 b=true}}"
        );

        let res = Test::deserialize(serialized.as_str());
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), test_struct);

        let anystruct = test_struct.as_any();
        let downcasted = anystruct.downcast_ref::<Test>().unwrap();
        assert_eq!(*downcasted, test_struct);
        println!("{:?}", downcasted);
    }
}
