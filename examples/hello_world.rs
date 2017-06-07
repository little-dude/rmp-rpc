extern crate futures;
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate rmp_rpc;


use tokio_service::{NewService, Service};
use rmp_rpc::{serve, Client, Message, Value, Utf8String};
use tokio_core::reactor::Core;
use std::{io, thread};
use std::time::Duration;
use futures::{future, Future};


#[derive(Clone)]
pub struct HelloWorld;

impl NewService for HelloWorld {
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Instance = HelloWorld;

    fn new_service(&self) -> io::Result<Self::Instance> {
        println!("server: new_service called.");
        Ok(self.clone())
    }
}

impl Service for HelloWorld {
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Future = Box<Future<Item = Message, Error = io::Error>>;

    fn call(&self, msg: Message) -> Self::Future {
        Box::new(
            match msg {
                Message::Request { method, .. } => {
                    match method.as_str() {
                        "hello" => future::ok(Message::Response {
                            result: Ok(Value::String(Utf8String::from("hello"))),
                            id: 0
                        }),
                        "world" => future::ok(Message::Response {
                            result: Ok(Value::String(Utf8String::from("world"))),
                            id: 0
                        }),
                        method => future::ok(Message::Response {
                            result: Err(Value::String(Utf8String::from(format!("unknown method {}", method).as_str()))),
                            id: 0
                        }),
                    }
                }
                _ => unimplemented!()
            }
        )
    }
}

fn main() {
    let addr = "127.0.0.1:54321".parse().unwrap();

    thread::spawn(move || {
        serve(addr, HelloWorld)
    });

    thread::sleep(Duration::from_millis(100));

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    core.run(
        Client::connect(&addr, &handle)
            .and_then(|client| {
                println!("Client connected");
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
