extern crate futures;
extern crate tokio_core;
extern crate tokio_proto;
extern crate rmp_rpc;


use tokio_proto::TcpServer;
use rmp_rpc::server::{Server, Dispatch};
use rmp_rpc::client::Client;
use rmp_rpc::protocol::Protocol;
use rmp_rpc::msgpack::{Value, Utf8String};
use tokio_core::reactor::Core;
use std::thread;
use std::time::Duration;
use futures::Future;


#[derive(Clone)]
pub struct HelloWorld;

impl Dispatch for HelloWorld {
    fn dispatch(&mut self, method: &str, _params: &[Value]) -> Result<Value, Value> {
        match method {
            "hello" => { Ok(Value::String(Utf8String::from("hello"))) }
            "world" => { Ok(Value::String(Utf8String::from("world"))) }
            _ => { Err(Value::String(Utf8String::from(format!("Invalid method {}", method)))) }
        }
    }
}

fn main() {
    let addr = "127.0.0.1:54321".parse().unwrap();

    thread::spawn(move || {
        let tcp_server = TcpServer::new(Protocol, addr);
        tcp_server.serve(|| {
            Ok(Server::new(HelloWorld))
        });
    });

    thread::sleep(Duration::from_millis(100));

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    core.run(
        Client::connect(&addr, &handle)
            .and_then(|client| {
                client.request("hello", vec![])
                    .and_then(move |response| {
                        println!("client: {:?}", response);
                        client.request("world", vec![])
                    })
                    .and_then(|response| {
                        println!("client: {:?}", response);
                        Ok(())
                    })
            })).unwrap();
}
