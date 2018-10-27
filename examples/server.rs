//! Here is an simple example with a pure server that that provides a method ``do_long_computation`
//! that simulates a long computation. This server can be tested with the following python script
//! (which requires [`msgpack-rpc-python`](https://github.com/msgpack-rpc/msgpack-rpc-python):
//!
//! ```python
//! import time
//! import msgpackrpc
//!
//! client = client = msgpackrpc.Client(msgpackrpc.Address("127.0.0.1", 54321))
//! start = time.time()
//! requests = []
//! for i in range(0, 1000):
//!     requests.append(client.call_async('do_long_computation', 5))
//! for req in requests:
//!     req.get()
//! end = time.time()
//! print(end - start)
//! ```
//#![feature(futures_api)]
extern crate chrono;
extern crate env_logger;
extern crate futures;
extern crate rmp_rpc;
extern crate tokio;
#[macro_use]
extern crate log;
use std::net::SocketAddr;
use std::time;

use futures::{future, Future, Poll, Stream};
use rmp_rpc::{serve, Service, Value};
use tokio::net::TcpListener;
use tokio::timer::Delay;

/// Our server type
#[derive(Clone)]
pub struct Server;

/// A future that does nothing but simulates a long computation and returns the time at which is
/// finishes in seconds.
pub struct LongComputation(Delay);

impl Future for LongComputation {
    type Item = Value;
    type Error = Value;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        info!("polling LongComputation");
        self.0
            .poll()
            .map(|future| {
                future.map(|()| {
                    time::SystemTime::now()
                        .duration_since(time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        .into()
                })
            })
            .map_err(|_| "error".into())
    }
}

/// The Service trait defines how the server handles incoming requests and notifications.
impl Service for Server {
    type RequestFuture = Box<Future<Item = Value, Error = Value> + Send>;

    /// Define how the server handle requests. This server accept requests with the method
    /// "do_long_computation" and an integer as parameter. It waits for the number of seconds specified in the parameter, and then sends back the server's time in seconds.
    fn handle_request(&mut self, method: &str, params: &[Value]) -> Self::RequestFuture {
        if method != "do_long_computation" {
            return Box::new(future::err(format!("Invalid method {}", method).into()));
        }
        if params.len() != 1 {
            return Box::new(future::err(
                "'do_long_computation' takes one argument".into(),
            ));
        }
        if let Value::Integer(ref value) = params[0] {
            if let Some(value) = value.as_u64() {
                return Box::new(LongComputation(Delay::new(
                    time::Instant::now() + time::Duration::from_secs(value),
                )));
            }
        }
        Box::new(future::err("Argument must be an unsigned integer".into()))
    }

    /// Define how the server handle notifications. This server just prints the method in the
    /// console.
    fn handle_notification(&mut self, method: &str, _: &[Value]) {
        println!("{}", method);
    }
}

fn main() {
    env_logger::init();
    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    // Create a listener to listen for incoming TCP connections.
    let server = TcpListener::bind(&addr)
        .unwrap()
        .incoming()
        // Each time the listener finds a new connection, start up a server to handle it.
        .map_err(|e| info!("error on TcpListener: {}", e))
        .for_each(move |stream| {
            info!("new connection {:?}", stream);
            info!("spawning a new Server");
            serve(stream, Server).map_err(|e| info!("server error {}", e))
        });

    // Run the server on the tokio event loop. This is blocking. Press ^C to stop
    tokio::run(server);
}
