//! This example shows how to have endpoints that act both as client and server.
//!
//! A "client" endpoint sends "ping" requests.
//! Upon receiving a "ping", the "server" endpoint sends a "pong" (it then acts as a client).
//! Upon receive a "pong", the "client" endpoint increments a counter (it then acts as a server).
//!
//! In this example, the client sends 10 pings, so we expect the pong counter to be 10.
//!
extern crate env_logger;
extern crate futures;
#[macro_use]
extern crate log;
extern crate rmp_rpc;
extern crate tokio_core;

use std::net::SocketAddr;
use std::io;
use std::sync::{Arc, Mutex};

use futures::{future, Future};
use tokio_core::reactor::Core;

use rmp_rpc::{serve, Client, Connector, Service, ServiceBuilder, Value};

// Our endpoint type
#[derive(Clone)]
pub struct PingPong {
    // Number of "pong" received
    pub value: Arc<Mutex<i64>>,
    // Our type is both a client and server, so we need to store the client
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

// Implement how the endpoint handles incoming requests and notifications.
// In this example, the endpoint does not handle notifications.
impl Service for PingPong {
    type T = String;
    type E = String;
    type Error = io::Error;

    fn handle_request(
        &mut self,
        method: &str,
        params: &[Value],
    ) -> Box<Future<Item = Result<Self::T, Self::E>, Error = Self::Error>> {
        let client = self.client.clone();
        match method {
            // Upon receiving a "ping", send a "pong" back. Note that the future we return
            // finishes only when the "pong" has been answered.
            "ping" => {
                let id = params[0].as_i64().unwrap();
                info!("received ping({}), sending pong", id);
                let request = client
                    .unwrap()
                    .request("pong", &[id.into()])
                    .and_then(|_result| Ok(Ok(String::new())))
                    .map_err(|()| {
                        io::Error::new(io::ErrorKind::Other, "The pong request failed")
                    });

                // The response is the result of the an empty string (quite silly but it's just for
                // the example).
                Box::new(request)
            }
            // Upon receiving a "pong" increment our pong counter.
            "pong" => {
                let id = params[0].as_i64().unwrap();
                info!("received pong({}), incrementing pong counter", id);
                *self.value.lock().unwrap() += 1;
                Box::new(future::ok(Ok(String::new())))
            }
            method => {
                let err = Err(format!("Invalid method {}", method));
                Box::new(future::ok(err))
            }
        }
    }

    fn handle_notification(
        &mut self,
        _: &str,
        _: &[Value],
    ) -> Box<Future<Item = (), Error = Self::Error>> {
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

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    // Spawn a server on the Tokio event loop
    handle.spawn(serve(addr, PingPong::new(), handle.clone()));

    let ping_pong_client = PingPong::new();
    core.run(
        Connector::new(&addr, &handle)
            // We want the client to handle the "pong" requests, so we have to set a service
            // builder. The service will be created during the connection.
            .set_service_builder(ping_pong_client.clone())
            .connect()
            .or_else(|e| {
                error!("Connection to server failed: {}", e);
                Err(())
            })
            .and_then(|client| {
                let mut requests = vec![];
                for i in 0..10 {
                    requests.push(
                        client
                            .request("ping", &[i.into()])
                            .and_then(|_response| Ok(())),
                    );
                }
                future::join_all(requests)
            }),
    ).unwrap();
    info!("Received {} pongs", ping_pong_client.value.lock().unwrap());
}
