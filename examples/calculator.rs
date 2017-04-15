extern crate tokio_proto;
extern crate rmp_rpc;

use tokio_proto::TcpServer;
use rmp_rpc::{Server, Protocol, Dispatch};
use rmp_rpc::msgpack::{Value, Utf8String, Integer};

fn argument_error() -> Value {
    Value::String(Utf8String::from("Invalid arguments"))
}


#[derive(Clone)]
pub struct Calculator {
    value: i64,
}

impl Calculator {
    fn new() -> Self {
        Calculator { value: 0 }
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

    fn clear(&mut self) -> Result<Value, Value> {
        self.value = 0;
        self.res()
    }

    fn res(&self) -> Result<Value, Value> {
        Ok(Value::Integer(Integer::from(self.value)))
    }

    fn add(&mut self, params: &[Value]) -> Result<Value, Value> {
        self.value += Self::parse_args(params)?
            .iter()
            .fold(0, |acc, &int| acc + int);
        self.res()
    }

    fn sub(&mut self, params: &[Value]) -> Result<Value, Value> {
        self.value -= Self::parse_args(params)?
            .iter()
            .fold(0, |acc, &int| acc - int);
        self.res()
    }
}

impl Dispatch for Calculator {
    fn dispatch(&mut self, method: &str, params: &[Value]) -> Result<Value, Value> {
        match method {
            "add" | "+" => { self.add(params) },
            "sub" | "-" => { self.sub(params) },
            "res" | "=" => { self.res() },
            "clear" => { self.clear() },
            _ => {
                Err(Value::String(Utf8String::from(format!("Invalid method {}", method))))
            }
        }
    }
}

fn main() {
    let addr = "127.0.0.1:54321".parse().unwrap();
    let tcp_server = TcpServer::new(Protocol, addr);
    tcp_server.serve(|| { Ok(Server::new(Calculator::new())) });
}
