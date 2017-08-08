extern crate env_logger;
extern crate futures;
extern crate rmp_rpc;
extern crate tokio_core;

use std::{io, thread};
use std::time::Duration;
use std::net::SocketAddr;

use rmp_rpc::{serve, Connector, Service, ServiceBuilder, Value, Client};
use tokio_core::reactor::Core;
use std::sync::{Arc, Mutex};

use futures::{future, Future};

#[derive(Clone)]
pub struct PingPong {
    pub value: Arc<Mutex<i64>>,
    pub client: Option<Client>,
}

impl PingPong {
    fn new() -> Self {
        PingPong {
            value: Arc::new(Mutex::new(0)),
            client: None,
        }
    }
}

impl Service for PingPong {
    type T = String;
    type E = String;
    type Error = io::Error;

    fn handle_request(&mut self, method: &str, _: &[Value]) -> Box<Future<Item = Result<Self::T, Self::E>, Error = Self::Error>> {
        let client = self.client.clone();
        match method {
            "ping" => {
                return Box::new(client.unwrap()
                    .request("pong", &[])
                    .and_then(|_result| Ok(Ok(String::new())))
                    .map_err(|()| { io::Error::new(io::ErrorKind::Other, "The pong request failed") }))
            }
            "pong" => {
                *self.value.lock().unwrap() += 1;
                return Box::new(future::ok(Ok(String::new())));
            }
            method => {
                let err = Err(format!("Invalid method {}", method));
                return Box::new(future::ok(err));
            }
        }
    }

    fn handle_notification(&mut self, _: &str, _: &[Value],) -> Box<Future<Item = (), Error = Self::Error>> {
        unimplemented!();
    }
}

impl ServiceBuilder for PingPong {
    type Service = PingPong;

    fn build(&self, client: Client) -> Self::Service {
        PingPong {
            value: self.value.clone(),
            client: Some(client),
        }
    }
}

fn main() {
    env_logger::init().unwrap();
    thread::spawn(|| {
        let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
        serve(&addr.clone(), PingPong::new())
    });
    thread::sleep(Duration::from_millis(100));

    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let ping_pong_client = PingPong::new();
    core.run(
        Connector::new(&addr, &handle)
            .set_service_builder(ping_pong_client.clone())
            .connect()
            .or_else(|e| {
                println!("Connection to server failed: {}", e);
                Err(())
            })
            .and_then(|client| {
                client
                    .request("ping", &[])
                    .and_then(|_response| {
                        Ok(())
                    })
            })).unwrap();
    println!("{}", ping_pong_client.value.lock().unwrap());
}
