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
use std::sync::{Arc, Mutex};

use futures::{future, Future, Stream};
use tokio_core::reactor::Core;
use tokio_core::net::{TcpListener, TcpStream};

use rmp_rpc::{Client, Endpoint, ServiceWithClient, Value};

// Our endpoint type
#[derive(Clone)]
pub struct PingPong {
    // Number of "pong" received
    pub value: Arc<Mutex<i64>>,
}

impl PingPong {
    fn new() -> Self {
        PingPong {
            value: Arc::new(Mutex::new(0)),
        }
    }
}

// Implement how the endpoint handles incoming requests and notifications.
// In this example, the endpoint does not handle notifications.
impl ServiceWithClient for PingPong {
    type RequestFuture = Box<Future<Item = Value, Error = Value> + 'static>;
    type NotificationFuture = Result<(), ()>;

    fn handle_request(
        &mut self,
        client: &mut Client,
        method: &str,
        params: &[Value],
    ) -> Self::RequestFuture {
        match method {
            // Upon receiving a "ping", send a "pong" back. Only after we get a response back from
            // "pong", we return the empty string.
            "ping" => {
                let id = params[0].as_i64().unwrap();
                info!("received ping({}), sending pong", id);
                let request = client
                    .request("pong", &[id.into()])
                    // After we get the "pong" back, send back an empty string.
                    .and_then(|_| Ok("".into()))
                    .map_err(|_| "".into());

                Box::new(request)
            }
            // Upon receiving a "pong" increment our pong counter and send the empty string back
            // immediately.
            "pong" => {
                let id = params[0].as_i64().unwrap();
                info!("received pong({}), incrementing pong counter", id);
                *self.value.lock().unwrap() += 1;
                Box::new(future::ok("".into()))
            }
            method => {
                let err = format!("Invalid method {}", method).into();
                Box::new(future::err(err))
            }
        }
    }

    fn handle_notification(
        &mut self,
        _: &mut Client,
        _: &str,
        _: &[Value],
    ) -> Self::NotificationFuture {
        unimplemented!();
    }
}

fn main() {
    env_logger::init().unwrap();

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let listener = TcpListener::bind(&addr, &handle).unwrap().incoming();
    // Spawn a "remote" endpoint on the Tokio event loop
    let handle_clone = handle.clone();
    handle.spawn(
        listener
            .for_each(move |(stream, _addr)| Endpoint::new(stream, PingPong::new(), &handle_clone))
            .map_err(|_| ()),
    );

    let ping_pong_client = PingPong::new();
    let pongs = ping_pong_client.value.clone();
    core.run(
        TcpStream::connect(&addr, &handle)
            .map_err(|_| ())
            .and_then(|stream| {
                // Make a "local" endpoint.
                let endpoint = Endpoint::new(stream, ping_pong_client, &handle);
                let client = endpoint.client();
                let mut requests = vec![];
                for i in 0..10 {
                    requests.push(
                        client
                            .request("ping", &[i.into()])
                            .and_then(|_response| Ok(())),
                    );
                }

                // Run all of the requests, along with the "local" endpoint.
                future::join_all(requests)
                    // The endpoint will never finish, so this is saying that we should terminate
                    // as soon as all of the requests are finished.
                    .select2(endpoint.map_err(|_| ()))
                    .map(|_| ())
                    .map_err(|_| ())
            }),
    ).unwrap();
    println!("Received {} pongs", pongs.lock().unwrap());
}
