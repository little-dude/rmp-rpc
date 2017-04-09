rmp-rpc
=======

A Rust implementation of MessagePack-RPC inspired by [`msgpack-rpc-rust`](https://github.com/euclio/msgpack-rpc-rust), but based on [tokio](http://tokio.rs/)

Example
-------

```rust
extern crate tokio_proto;
extern crate rmp_rpc;

use tokio_proto::TcpServer;
use rmp_rpc::{Server, Protocol, Dispatch};
use rmp_rpc::msgpack::{Value, Utf8String, Integer};


// A simple dispatcher that know only two methods, "hello" and "world"
#[derive(Clone)]
pub struct HelloWorld;

impl Dispatch for HelloWorld {
    fn dispatch(&self, method: &str, _params: &[Value]) -> Result<Value, Value> {
        match method {
            "hello" => { Ok(Value::String(Utf8String::from("hello"))) }
            "world" => { Ok(Value::String(Utf8String::from("world"))) }
            _ => { Err(Value::String(Utf8String::from(format!("Invalid method {}", method)))) }
        }
    }
}

fn main() {
    let addr = "127.0.0.1:54321".parse().unwrap();
    let tcp_server = TcpServer::new(Protocol, addr);
    // Instantiate a new server for each client.
    tcp_server.serve(|| { Ok(Server::new(HelloWorld{})) });
}

```
