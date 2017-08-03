//! In this example
extern crate futures;
extern crate tokio_core;
extern crate rmp_rpc;
extern crate log;
extern crate env_logger;

use std::marker::Send;
use std::{io, thread};
use std::time::Duration;
use std::net::SocketAddr;

use futures::{future, BoxFuture, Future};
use rmp_rpc::{serve, DefaultConnector, Service, ServiceBuilder, Value};
use tokio_core::reactor::Core;

#[derive(Clone)]
pub struct HelloWorld;

impl ServiceBuilder for HelloWorld {
    type Service = HelloWorld;

    fn build(&self) -> Self::Service {
        self.clone()
    }
}

fn box_ok<T: Send + 'static, E: Send + 'static>(t: T) -> BoxFuture<T, E> {
    Box::new(future::ok(t))
}

impl Service for HelloWorld {
    type Error = io::Error;
    type T = String;
    type E = String;

    fn handle_request(
        &mut self,
        method: &str,
        params: &[Value],
    ) -> BoxFuture<Result<Self::T, Self::E>, Self::Error> {
        if method != "hello" {
            return box_ok(Err(format!("Uknown method {}", method)));
        }

        if params.len() != 1 {
            return box_ok(Err(format!(
                "Expected 1 argument for method \"hello\", got {}",
                params.len()
            )));
        }

        if let Value::String(ref string) = params[0] {
            if let Some(name) = string.as_str() {
                return box_ok(Ok(format!("hello {}", name)));
            }
        }
        box_ok(Err("Invalid argument".into()))
    }

    fn handle_notification(
        &mut self,
        method: &str,
        _params: &[Value],
    ) -> BoxFuture<(), Self::Error> {
        // just pring the notification's method name
        box_ok(println!("{}", method))
    }
}

fn main() {
    env_logger::init().unwrap();
    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();

    thread::spawn(move || serve(&addr, &HelloWorld));
    thread::sleep(Duration::from_millis(100));

    let mut core = Core::new().unwrap();
    let handle = core.handle();


    let _ = core.run(
        DefaultConnector::new(&addr, &handle)
            .connect()
            .or_else(|e| {
                println!("Connection to server failed: {}", e);
                Err(())
            })
            .and_then(|client| {
                client.request("hello", &["little-dude".into()]).and_then(
                    |response| {
                        println!("{:?}", response);
                        client
                            .notify("this should be printed :)", &[])
                            .and_then(|_| Ok(client))
                    },
                )
            })
            .and_then(|client| {
                client.request("dummy", &[]).and_then(|response| {
                    println!("{:?}", response);
                    Ok(())
                })
            }),
    );
}
