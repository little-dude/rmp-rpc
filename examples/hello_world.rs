extern crate futures;
extern crate tokio_core;
extern crate rmp_rpc;
extern crate log;
extern crate env_logger;


use rmp_rpc::client::Client;
use rmp_rpc::server::{ServiceBuilder, Service, serve};
use rmp_rpc::{Request, Notification};

use tokio_core::reactor::Core;
use std::{io, thread};
use std::time::Duration;
use std::net::SocketAddr;
use futures::{future, Future, BoxFuture};


#[derive(Clone)]
pub struct HelloWorld;

impl ServiceBuilder for HelloWorld {
    type Service = HelloWorld;

    fn build(&self) -> Self::Service {
        self.clone()
    }
}

impl Service for HelloWorld {
    type Error = io::Error;
    type T = &'static str;
    type E = String;

    fn handle_request(
        &mut self,
        request: &Request,
    ) -> BoxFuture<Result<Self::T, Self::E>, Self::Error> {
        Box::new(match request.method.as_str() {
            "hello" => future::ok(Ok("hello")),
            "world" => future::ok(Ok("world")),
            method => future::ok(Err(format!("unknown method {}", method))),
        })
    }

    fn handle_notification(&mut self, _notification: &Notification) -> BoxFuture<(), Self::Error> {
        unimplemented!();
    }
}

fn main() {
    env_logger::init().unwrap();
    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();

    thread::spawn(move || serve(&addr, HelloWorld));
    thread::sleep(Duration::from_millis(100));

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let _ = core.run(
        Client::connect(&addr, &handle)
            .or_else(|e| {
                println!("Connection to server failed: {}", e);
                Err(())
            })
            .and_then(|client| {
                client.request("hello", &[]).and_then(|response| {
                    println!("{:?}", response);
                    client.request("dummy", &[]).and_then(|response| {
                        println!("{:?}", response);
                        Ok(client)
                    })
                })
            })
            .and_then(|client| {
                client.request("world", &[]).and_then(|response| {
                    println!("{:?}", response);
                    Ok(())
                })
            }),
    );
}
