//! This example shows how to have endpoints that act both as client and server.
//!
//! A "client" endpoint sends "ping" requests.
//! Upon receiving a "ping", the "server" endpoint sends a "pong" (it then acts as a client).
//! Upon receive a "pong", the "client" endpoint increments a counter (it then acts as a server).
//!
//! In this example, the client sends 10 pings, so we expect the pong counter to be 10.
//!
#[macro_use]
extern crate log;

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures::{future, Future, FutureExt, TryFutureExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::compat::TokioAsyncReadCompatExt;

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
    type RequestFuture = Pin<Box<dyn Future<Output = Result<Value, Value>> + 'static + Send>>;

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
                    .map_ok(|_| "".into())
                    .map_err(|_| "".into());

                Box::pin(request)
            }
            // Upon receiving a "pong" increment our pong counter and send the empty string back
            // immediately.
            "pong" => {
                let id = params[0].as_i64().unwrap();
                info!("received pong({}), incrementing pong counter", id);
                *self.value.lock().unwrap() += 1;
                Box::pin(future::ok("".into()))
            }
            method => {
                let err = format!("Invalid method {}", method).into();
                Box::pin(future::err(err))
            }
        }
    }

    fn handle_notification(&mut self, _: &mut Client, _: &str, _: &[Value]) {
        unimplemented!();
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::init();

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let listener = TcpListener::bind(&addr).await?;
    // Spawn a "remote" endpoint on the Tokio event loop
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((socket, _)) => {
                    tokio::spawn(Endpoint::new(socket.compat(), PingPong::new()));
                }
                Err(e) => debug!("Error accepting connection: {}", e),
            }
        }
    });

    let ping_pong_client = PingPong::new();
    let pongs = ping_pong_client.value.clone();

    let socket = TcpStream::connect(&addr).await?;
    // Make a "local" endpoint.
    let endpoint = Endpoint::new(socket.compat(), ping_pong_client);
    let client = endpoint.client();

    let mut requests = vec![];
    for i in 0..10 {
        requests.push(client.request("ping", &[i.into()]).map(|_response| ()));
    }

    // Run all of the requests, along with the "local" endpoint.
    // The endpoint will never finish, so this is saying that we should terminate
    // as soon as all of the requests are finished.
    future::select(future::join_all(requests), endpoint.map_err(|_| ())).await;
    println!("Received {} pongs", pongs.lock().unwrap());

    Ok(())
}
