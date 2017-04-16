extern crate tokio_core;
extern crate futures;
extern crate tokio_proto;
extern crate rmp_rpc;

use std::net::SocketAddr;
use tokio_proto::TcpServer;
use rmp_rpc::{Server, Protocol, Dispatch};
use rmp_rpc::msgpack::{Value, Utf8String, Integer};
use std::thread;
use futures::Future;
use tokio_core::reactor::{Handle, Core};
use std::io;
use std::time::Duration;

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

fn parse_response(value: Result<Value, Value>) ->  Result<Result<i64, String>, io::Error> {
    match value {
        Ok(res) => {
            if let Value::Integer(int) = res {
                match int.as_i64() {
                    Some(int) => {
                        return Ok(Ok(int));
                    }
                    None => {
                        return Ok(Err(format!("Could not parse server response as an integer")));
                    }
                }
            } else {
                Ok(Err(format!("Could not parse server response as an integer")))
            }
        }
        Err(err) => {
            if let Value::String(s) = err {
                match s.as_str() {
                    Some(err_str) => {
                        return Ok(Err(err_str.to_string()));
                    }
                    None => {
                        return Ok(Err(format!("Could not parse server response as a string")));
                    }
                }
            } else {
                Ok(Err(format!("Could not parse server response as a string")))
            }
        }
    }
}

pub type Response = Box<Future<Item=Result<i64, String>, Error=io::Error>>;

struct Client(rmp_rpc::Client);

impl Client {
    fn new(addr: &SocketAddr, handle: &Handle) -> Self {
        Client(rmp_rpc::Client::connect(addr, handle).wait().unwrap())
    }

    fn add(&self, values: &[i64]) -> Response {
        let res = self.0
            .call("add", values.iter().map(|v| Value::Integer(Integer::from(*v))).collect())
            .and_then(|response| parse_response(response));
        Box::new(res)
    }

    fn sub(&self, values: &[i64]) -> Response {
        let res = self.0
            .call("sub", values.iter().map(|v| Value::Integer(Integer::from(*v))).collect())
            .and_then(|response| parse_response(response));
        Box::new(res)
    }

    fn res(&self) -> Response {
        let res = self.0
            .call("res", vec![])
            .and_then(|response| parse_response(response));
        Box::new(res)
    }

    fn clear(&self) -> Response {
        let res = self.0
            .call("clear", vec![])
            .and_then(|response| parse_response(response));
        Box::new(res)
    }
}


fn main() {

    let addr = "127.0.0.1:12345".parse().unwrap();

    thread::spawn(move || {
        let tcp_server = TcpServer::new(Protocol, addr);
        tcp_server.serve(|| {
            Ok(Server::new(Calculator::new()))
        });
    });

    thread::sleep(Duration::from_millis(100));

    let core = Core::new().unwrap();
    let handle = core.handle();

    let client = Client::new(&addr, &handle);
    println!("{:?}", client.add(&vec![1,2,3]).wait());
    println!("{:?}", client.sub(&vec![1]).wait());
    println!("{:?}", client.res().wait());
    println!("{:?}", client.clear().wait());
}
