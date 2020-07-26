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
use env_logger;

use tokio;
#[macro_use]
extern crate log;

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time;

use futures::{future, Future, TryFutureExt};
use rmp_rpc::{serve, Service, Value};
use tokio::net::TcpListener;
use tokio::time::{delay_until, Delay};
use tokio_util::compat::Tokio02AsyncReadCompatExt;

/// Our server type
#[derive(Clone)]
pub struct Server;

/// A future that does nothing but simulates a long computation and returns the time at which is
/// finishes in seconds.
pub struct LongComputation(Delay);

impl Future for LongComputation {
    type Output = Result<Value, Value>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        info!("polling LongComputation");
        Pin::new(&mut self.0)
            .poll(cx)
            .map(|()| {
                Ok(time::SystemTime::now()
                    .duration_since(time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .into())
            })
    }
}

/// The Service trait defines how the server handles incoming requests and notifications.
impl Service for Server {
    type RequestFuture = Pin<Box<dyn Future<Output = Result<Value, Value>> + Send>>;

    /// Define how the server handle requests. This server accept requests with the method
    /// "do_long_computation" and an integer as parameter. It waits for the number of seconds specified in the parameter, and then sends back the server's time in seconds.
    fn handle_request(&mut self, method: &str, params: &[Value]) -> Self::RequestFuture {
        if method != "do_long_computation" {
            return Box::pin(future::err(format!("Invalid method {}", method).into()));
        }
        if params.len() != 1 {
            return Box::pin(future::err(
                "'do_long_computation' takes one argument".into(),
            ));
        }
        if let Value::Integer(ref value) = params[0] {
            if let Some(value) = value.as_u64() {
                return Box::pin(LongComputation(delay_until(
                    (time::Instant::now() + time::Duration::from_secs(value)).into(),
                )));
            }
        }
        Box::pin(future::err("Argument must be an unsigned integer".into()))
    }

    /// Define how the server handle notifications. This server just prints the method in the
    /// console.
    fn handle_notification(&mut self, method: &str, _: &[Value]) {
        println!("{}", method);
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::init();
    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    // Create a listener to listen for incoming TCP connections.
    let mut listener = TcpListener::bind(&addr).await?;
    loop {
        let socket = match listener.accept().await {
            Ok((socket, _)) => socket,
            Err(e) => {
                info!("error on TcpListener: {}", e);
                continue
            },
        };
        info!("new connection {:?}", socket);
        info!("spawning a new Server");
        // Important! The server must be spawned in the background! Otherwise, our server will
        // wait for each connection to be processed before accepting a new one.
        tokio::spawn(serve(socket.compat(), Server).map_err(|e| info!("server error {}", e)));
    }
}
