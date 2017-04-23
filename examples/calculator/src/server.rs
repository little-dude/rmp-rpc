use std::sync::{Arc, Mutex};
use std::io;
use rmp_rpc::{Message, Value, Utf8String, Integer};
use futures::{future, Future};
use tokio_service::{Service, NewService};

fn argument_error() -> Value {
    Value::String(Utf8String::from("Invalid arguments"))
}


#[derive(Clone)]
pub struct Calculator {
    value: Arc<Mutex<i64>>,
}

impl Calculator {
    pub fn new() -> Self {
        Calculator { value: Arc::new(Mutex::new(0)) }
    }

    fn parse_args(params: &[Value]) -> Result<Vec<i64>, Value> {
        let mut ret = vec![];
        for value in params {
            let int = if let Value::Integer(int) = *value {
                int.as_i64().ok_or(argument_error())?
            } else {
                return Err(argument_error());
            };
            ret.push(int);
        }
        Ok(ret)
    }

    fn clear(&self) -> Result<Value, Value> {
        println!("server: clear");
        let mut value = self.value.lock().unwrap();
        *value = 0;
        Ok(Value::Integer(Integer::from(*value)))
    }

    fn res(&self) -> Result<Value, Value> {
        println!("server: res");
        Ok(Value::Integer(Integer::from(*self.value.lock().unwrap())))
    }

    fn add(&self, params: &[Value]) -> Result<Value, Value> {
        println!("server: add");
        let mut value = self.value.lock().unwrap();
        *value += Self::parse_args(params)?.iter().fold(0, |acc, &int| acc + int);
        Ok(Value::Integer(Integer::from(*value)))
    }

    fn sub(&self, params: &[Value]) -> Result<Value, Value> {
        println!("server: sub");
        let mut value = self.value.lock().unwrap();
        *value -= Self::parse_args(params)?.iter().fold(0, |acc, &int| acc + int);
        Ok(Value::Integer(Integer::from(*value)))
    }
}

impl Service for Calculator {
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Future = Box<Future<Item = Message, Error = io::Error>>;

    fn call(&self, msg: Message) -> Self::Future {
        let res = match msg {
            Message::Request { method, params, .. } => {
                match method.as_str() {
                    "add" | "+" => self.add(&params),
                    "sub" | "-" => self.sub(&params),
                    "res" | "=" => self.res(),
                    "clear" => self.clear(),
                    method => {
                        Err(Value::String(Utf8String::from(format!("Invalid method {}", method))))
                    }
                }
            }
            _ => unimplemented!(),
        };
        Box::new(future::ok(Message::Response {
            result: res,
            id: 0,
        }))
    }
}

impl NewService for Calculator {
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Instance = Calculator;

    fn new_service(&self) -> io::Result<Self::Instance> {
        println!("server: new_service called.");
        Ok(self.clone())
    }
}
