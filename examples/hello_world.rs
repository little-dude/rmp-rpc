extern crate futures;
extern crate tokio_core;
extern crate rmp_rpc;


use rmp_rpc::{ServiceBuilder, Service, serve, Client, Response, Request, Notification, Value,
              Utf8String};
use tokio_core::reactor::Core;
use std::{io, thread};
use std::time::Duration;
use std::net::SocketAddr;
use futures::{future, Future};


#[derive(Clone)]
pub struct HelloWorld;

impl ServiceBuilder for HelloWorld {
    type Service = HelloWorld;

    fn build(&self) -> Self::Service {
        println!("server: new_service called.");
        self.clone()
    }
}

impl Service for HelloWorld {
    type Error = io::Error;

    fn handle_request(
        &mut self,
        request: &Request,
    ) -> Box<Future<Item = Result<Response, Self::Error>, Error = io::Error>> {
        Box::new(match request.method.as_str() {
            "hello" => future::ok(Ok(Response {
                result: Ok(Value::String(Utf8String::from("hello"))),
                id: 0,
            })),
            "world" => future::ok(Ok(Response {
                result: Ok(Value::String(Utf8String::from("world"))),
                id: 0,
            })),
            method => future::ok(Ok(Response {
                result: Err(Value::String(Utf8String::from(
                    format!("unknown method {}", method).as_str(),
                ))),
                id: 0,
            })),
        })
    }

    fn handle_notification(
        &mut self,
        _notification: &Notification,
    ) -> Box<Future<Item = Result<(), Self::Error>, Error = io::Error>> {
        unimplemented!();
    }
}

fn main() {
    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();

    thread::spawn(move || serve(&addr, HelloWorld));
    thread::sleep(Duration::from_millis(100));

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    core.run(Client::connect(&addr, &handle).and_then(|client| {
        println!("Client connected");
        client
            .request("hello", vec![])
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
