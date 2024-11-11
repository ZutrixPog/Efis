mod dispatcher;

use std::sync::Arc;

pub mod client;
pub mod server;

pub trait SerDe: Serialize + Deserialize + Clone + Send + Sync {
    fn as_any(&self) -> Arc<dyn std::any::Any>;
}

pub trait Serialize {
    fn serialize(&self) -> String;
}

pub trait Deserialize: Sized {
    fn deserialize(s: &str) -> Result<Self, String>;
}

pub trait RpcStruct {
    fn register_fns(&'static self, dispatcher: &mut dispatcher::Dispatcher);
}

#[cfg(test)]
mod tests {
    use super::*;
    use macros::SerDe;

    #[derive(SerDe, Debug, PartialEq, Default, Clone)]
    struct Test {
        a: i32,
        b: f64,
        c: String,
    }

    #[test]
    fn test_macro() {
        let test_struct = Test {
            a: 2,
            b: 0.2,
            c: String::from("hey"),
        };

        let serialized = test_struct.serialize();
        assert_eq!(serialized, "2 0.2 hey");

        let dt = Test {
            ..Default::default()
        };
        let mut res = Test::deserialize(serialized.as_str());
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), test_struct);

        let anystruct = test_struct.as_any();
        let downcasted = anystruct.downcast_ref::<Test>().unwrap();
        assert_eq!(*downcasted, test_struct);
        println!("{:?}", downcasted);
    }
}
