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
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};

use futures::{future, Future, Stream};
use tokio_core::reactor::Core;
use tokio_core::net::{TcpListener, TcpStream};

use rmp_rpc::{Client, Endpoint, Service, Value};

// Our endpoint type
#[derive(Clone)]
pub struct PingPong {
    // Number of "pong" received
    pub value: Arc<Mutex<i64>>,
    pub client: Arc<Mutex<Option<Client>>>,
}

impl PingPong {
    fn new() -> Self {
        PingPong {
            value: Arc::new(Mutex::new(0)),
            client: Arc::new(Mutex::new(None)),
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
        match method {
            // Upon receiving a "ping", send a "pong" back. Note that the future we return
            // finishes only when the "pong" has been answered.
            "ping" => {
                let id = params[0].as_i64().unwrap();
                info!("received ping({}), sending pong", id);
                let request =
                    self.client.lock().unwrap().deref_mut().as_mut().unwrap()
                    .request("pong", &[id.into()])
                    .and_then(|_result| Ok(Ok(String::new())))
                    .map_err(|()| io::Error::new(io::ErrorKind::Other, "The pong request failed"));

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

fn main() {
    env_logger::init().unwrap();

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let listener = TcpListener::bind(&addr, &handle).unwrap().incoming();
    // Spawn a "remote" server on the Tokio event loop
    handle.spawn(
        listener
        .for_each(|(stream, _addr)| {
            let remote_server = PingPong::new();
            let remote_client_ref = remote_server.client.clone();
            let (server, client) = Endpoint::new(stream, remote_server);
            {
                *(remote_client_ref.lock().unwrap().deref_mut()) = Some(client.clone());
            }
            server
        })
        .map_err(|_| ())
    );

    let ping_pong_client = PingPong::new();
    let pongs = ping_pong_client.value.clone();
    core.run(
        TcpStream::connect(&addr, &handle)
            .map_err(|_| ())
            .and_then(|stream| {
                // Make a "local" client/server pair.
                let (server, client) = Endpoint::new(stream, ping_pong_client);
                let mut requests = vec![];
                for i in 0..10 {
                    requests.push(
                        client
                            .request("ping", &[i.into()])
                            .and_then(|_response| Ok(())),
                    );
                }

                // Run all of the requests, along with the "local" server.
                future::join_all(requests).join(server.map_err(|_| ()))
            })
    );
    info!("Received {} pongs", pongs.lock().unwrap());
}

