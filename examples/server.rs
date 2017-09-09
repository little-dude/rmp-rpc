//! Here is an simple example with a pure server. A "pure" server cannot send requests or
//! notifications, and only handles incoming requests and notifications. `rmp-rpc` makes it
//! possible to have a server that also act as a client and is able to send requests and
//! notifications to the remote endpoint.
extern crate futures;
extern crate rmp_rpc;
extern crate tokio_core;

use std::io;
use std::marker::Send;
use std::net::SocketAddr;

use futures::{future, Future};
use rmp_rpc::{serve, Client, Service, ServiceBuilder, Value};
use tokio_core::reactor::Core;

// Our server type
#[derive(Clone)]
pub struct Echo;

// A little helper to make `handle_request()` a little less verbose
fn box_ok<T: Send + 'static, E: Send + 'static>(t: T) -> Box<Future<Item = T, Error = E>> {
    Box::new(future::ok(t))
}

// The Service trait defines how the server handles incoming requests and notifications.
impl Service for Echo {
    type Error = io::Error;

    // When a request succeeds, the response is a String.
    type T = String;
    // When a request fails, the error is a String.
    type E = String;

    // Define how the server handle requests.
    //
    // This server accept requests with the method "echo".
    // It echoes back the first parameter.
    // If the method is not echo, or if the first parameter is not a string, it returns an error.
    fn handle_request(
        &mut self,
        method: &str,
        params: &[Value],
    ) -> Box<Future<Item = Result<Self::T, Self::E>, Error = Self::Error>> {
        // If the method is not "echo", return an error.
        if method != "echo" {
            return box_ok(Err(format!("Uknown method {}", method)));
        }

        // Take the first parameter, which should be a string, and echo it back
        if let Value::String(ref string) = params[0] {
            if let Some(text) = string.as_str() {
                return box_ok(Ok(text.into()));
            }
        }

        // If we reach this point, return an error, that means the first parameter is not a String.
        box_ok(Err("Invalid argument".into()))
    }

    // Define how the server handle notifications.
    //
    // This server just prints the method in the console.
    fn handle_notification(
        &mut self,
        method: &str,
        _: &[Value],
    ) -> Box<Future<Item = (), Error = Self::Error>> {
        box_ok(println!("{}", method))
    }
}

// A new instance of our service will be created for each connection.
// The ServiceBuilder trait defines how this creation happens.
impl ServiceBuilder for Echo {
    type Service = Echo;

    fn build(&self, _client: Client) -> Self::Service {
        self.clone()
    }
}


fn main() {
    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    // Create a tokio event loop
    let mut core = Core::new().unwrap();
    // serve returns a future that needs to be spawned on the event loop.
    let server = serve(addr, Echo, core.handle());
    // Run the server. This is blocking. Press ^C to stop
    core.run(server).unwrap();
}
