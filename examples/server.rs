//! Here is an simple example with a pure server. A "pure" server cannot send requests or
//! notifications, and only handles incoming requests and notifications. `rmp-rpc` makes it
//! possible to have a server that also act as a client and is able to send requests and
//! notifications to the remote endpoint.
extern crate futures;
extern crate rmp_rpc;
extern crate tokio_core;

use std::net::SocketAddr;

use futures::Stream;
use rmp_rpc::{serve, Service, Value};
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;

// Our server type
#[derive(Clone)]
pub struct Echo;

// The Service trait defines how the server handles incoming requests and notifications.
impl Service for Echo {
    // This is the type of future we send back from `handle_request`. Since we have a response
    // available immediately, there's no need to send back a "genuine" future.
    type RequestFuture = Result<Value, Value>;

    // Define how the server handle requests.
    //
    // This server accept requests with the method "echo".
    // It echoes back the first parameter.
    // If the method is not echo, or if the first parameter is not a string, it returns an error.
    fn handle_request(&mut self, method: &str, params: &[Value]) -> Self::RequestFuture {
        // If the method is not "echo", return an error. Note that we "return" by sending back a
        // message on the return channel. If we wanted to, we could spawn some long-running
        // computation on another thread, and have that computation be in charge of sending back
        // the result.
        if method != "echo" {
            return Err(format!("Unknown method {}", method).into());
        }

        // Take the first parameter, which should be a string, and echo it back
        if let Value::String(ref string) = params[0] {
            if let Some(text) = string.as_str() {
                return Ok(text.into());
            }
        }

        // If we reach this point, return an error, that means the first parameter is not a String.
        Err("Invalid argument".into())
    }

    // Define how the server handle notifications.
    //
    // This server just prints the method in the console.
    fn handle_notification(&mut self, method: &str, _: &[Value]) {
        println!("{}", method);
    }
}

fn main() {
    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    // Create a tokio event loop.
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    // Create a listener to listen for incoming TCP connections.
    let server = TcpListener::bind(&addr, &handle)
        .unwrap()
        .incoming()
        // Each time the listener finds a new connection, start up a server to handle it.
        .for_each(move |(stream, _addr)| {
            serve(stream, Echo)
        });

    // Run the server on the tokio event loop. This is blocking. Press ^C to stop
    core.run(server).unwrap();
}
