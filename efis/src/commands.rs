use crate::{
    efis::types::{ExpireReq, GetReq, ListReq, MapReq, PubReq, SetReq},
    rpc::{Deserialize, Serialize},
};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Command {
    Set(SetReq),
    Delete(GetReq),
    Increment(GetReq),
    Decrement(GetReq),
    Expire(ExpireReq),
    Lpush(ListReq),
    Lpop(GetReq),
    Rpush(ListReq),
    Rpop(GetReq),
    Sadd(ListReq),
    Zadd(MapReq),
    Publish(PubReq),
    Unknown,
}

impl Serialize for Command {
    fn serialize(&self) -> String {
        match self {
            Command::Set(req) => format!("Set({})", req.serialize()),
            Command::Delete(req) => format!("Delete({})", req.serialize()),
            Command::Increment(req) => format!("Increment({})", req.serialize()),
            Command::Decrement(req) => format!("Decrement({})", req.serialize()),
            Command::Expire(req) => format!("Expire({})", req.serialize()),
            Command::Lpush(req) => format!("Lpush({})", req.serialize()),
            Command::Lpop(req) => format!("Lpop({})", req.serialize()),
            Command::Rpush(req) => format!("Rpush({})", req.serialize()),
            Command::Rpop(req) => format!("Rpop({})", req.serialize()),
            Command::Sadd(req) => format!("Sadd({})", req.serialize()),
            Command::Zadd(req) => format!("Zadd({})", req.serialize()),
            Command::Publish(req) => format!("Publish({})", req.serialize()),
            Command::Unknown => "Unknown".into(),
        }
    }
}

impl Deserialize for Command {
    fn deserialize(s: &str) -> Result<Self, String> {
        let s = s.trim();

        if !s.contains('(') {
            return match s {
                "Unknown" => Ok(Command::Unknown),
                "Set" | "Delete" | "Increment" | "Decrement" | "Expire" | "Lpush" | "Lpop"
                | "Rpush" | "Rpop" | "Sadd" | "Zadd" | "Publish" => {
                    Err(format!("Variant '{}' requires payload", s))
                }
                _ => Ok(Command::Unknown),
            };
        }

        let start = s.find('(').ok_or("Missing '(' in enum")?;

        let variant = s[..start].trim();
        let payload = &s[start + 1..]; // skip '('

        let mut depth = 1usize;
        let mut end = None;

        for (i, ch) in payload.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        let end = end.ok_or("Unmatched parentheses in enum payload")?;
        let inner = &payload[..end]; // Inside the parentheses

        match variant {
            "Set" => Ok(Command::Set(SetReq::deserialize(inner)?)),
            "Delete" => Ok(Command::Delete(GetReq::deserialize(inner)?)),
            "Increment" => Ok(Command::Increment(GetReq::deserialize(inner)?)),
            "Decrement" => Ok(Command::Decrement(GetReq::deserialize(inner)?)),
            "Expire" => Ok(Command::Expire(ExpireReq::deserialize(inner)?)),
            "Lpush" => Ok(Command::Lpush(ListReq::deserialize(inner)?)),
            "Lpop" => Ok(Command::Lpop(GetReq::deserialize(inner)?)),
            "Rpush" => Ok(Command::Rpush(ListReq::deserialize(inner)?)),
            "Rpop" => Ok(Command::Rpop(GetReq::deserialize(inner)?)),
            "Sadd" => Ok(Command::Sadd(ListReq::deserialize(inner)?)),
            "Zadd" => Ok(Command::Zadd(MapReq::deserialize(inner)?)),
            "Publish" => Ok(Command::Publish(PubReq::deserialize(inner)?)),
            "Unknown" => Ok(Command::Unknown),
            _ => Ok(Command::Unknown),
        }
    }
}
