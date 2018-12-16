//! This example shows how to have endpoints that act both as client and server.
//!
//! A "client" endpoint sends "ping" requests.
//! Upon receiving a "ping", the "server" endpoint sends a "pong" (it then acts as a client).
//! Upon receive a "pong", the "client" endpoint increments a counter (it then acts as a server).
//!
//! In this example, the client sends 10 pings, so we expect the pong counter to be 10.
//!
use env_logger;

#[macro_use]
extern crate log;



use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures::{future, Future, Stream};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;

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
    type RequestFuture = Box<dyn Future<Item = Value, Error = Value> + 'static + Send>;

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

    fn handle_notification(&mut self, _: &mut Client, _: &str, _: &[Value]) {
        unimplemented!();
    }
}

fn main() {
    env_logger::init();

    let mut rt = Runtime::new().unwrap();

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let listener = TcpListener::bind(&addr).unwrap().incoming();
    // Spawn a "remote" endpoint on the Tokio event loop
    rt.spawn(
        listener
            .for_each(move |stream| Endpoint::new(stream, PingPong::new()))
            .map_err(|_| ()),
    );

    let ping_pong_client = PingPong::new();
    let pongs = ping_pong_client.value.clone();
    rt.block_on(
        TcpStream::connect(&addr)
            .map_err(|_| ())
            .and_then(|stream| {
                // Make a "local" endpoint.
                let endpoint = Endpoint::new(stream, ping_pong_client);
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
    )
    .unwrap();
    println!("Received {} pongs", pongs.lock().unwrap());
}
