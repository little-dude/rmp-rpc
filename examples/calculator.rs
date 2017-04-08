extern crate tokio_proto;
extern crate rmp_rpc;

use tokio_proto::TcpServer;
use rmp_rpc::server;
use rmp_rpc::msgpack::{Value, Utf8String, Integer};

#[derive(Clone)]
pub struct Calculator;

impl Calculator {
    fn add(a: u64, b: u64) -> u64 {
        a + b
    }
}

fn err(msg: &str) -> Value {
    Value::String(Utf8String::from(msg))
}

impl server::Dispatch for Calculator {
    fn dispatch(&self, method: &str, params: &[Value]) -> Result<Value, Value> {
        match method {
            "add" => {
                if params.len() != 2 {
                    return Err(err("Invalid arguments for `add`"));
                }

                let a = if let Value::Integer(a) = params[0] {
                    a.as_u64().ok_or(err("Invalid arguments for `add`"))?
                } else {
                    Err(err("Invalid arguments for `add`"))?
                };

                let b = if let Value::Integer(b) = params[1] {
                    b.as_u64().ok_or(err("Invalid arguments for `add`"))?
                } else {
                    Err(err("Invalid arguments for `add`"))?
                };

                Ok(Value::Integer(Integer::from(Self::add(a, b))))
            }
            _ => {
                Err(err(&format!("Invalid method {}", method)))
            }
        }
    }
}

fn main() {
    let addr = "127.0.0.1:54321".parse().unwrap();
    let tcp_server = TcpServer::new(server::Proto, addr);
    tcp_server.serve(|| {
        Ok(server::Server::new(Calculator{}))
    });
}
