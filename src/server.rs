//! Building blocks for building `MessagePack-RPC` servers.
//!
//! Here is a server that returns the string "world" when a "hello" request is received, and that
//! prints the method name of any notification it receives.
//!
//! ```rust,no_run
//! extern crate futures;
//! extern crate rmpv;
//! extern crate rmp_rpc;
//!
//! use std::io;
//! use std::net::SocketAddr;
//!
//! use futures::{future, BoxFuture};
//! use rmpv::Value;
//! use rmp_rpc::server::{Service, ServiceBuilder, serve};
//!
//! // Our server is dead simple and does not have any attribute
//! #[derive(Clone)]
//! struct ExampleServer;
//!
//! // The Service trait defines how the server will handle the requests and notifications it
//! // receives.
//! impl Service for ExampleServer {
//!     type Error = io::Error;
//!     type T = &'static str;
//!     type E = String;
//!
//!     fn handle_request(&mut self, method: &str, _params: &[Value] ) -> BoxFuture<Result<Self::T, Self::E>, Self::Error> {
//!         Box::new(match method {
//!             // return "world" if the request's method is "hello"
//!             "hello" => future::ok(Ok("world")),
//!             // otherwise, return an error
//!             method => future::ok(Err(format!("unknown method {}", method))),
//!         })
//!     }
//!
//!     fn handle_notification(&mut self, method: &str, _params: &[Value]) -> BoxFuture<(), Self::Error> {
//!         // just pring the notification's method name
//!         Box::new(future::ok(println!("{}", method)))
//!     }
//! }
//!
//! // Since a new instance of our server is created for each client, we need to have a builder
//! // type that builds ExampleServer instances. In this case, our builder type is ExampleServer
//! // itself, but it could be any other type.
//! impl ServiceBuilder for ExampleServer {
//!     type Service = ExampleServer;
//!
//!     fn build(&self) -> Self::Service { self.clone() }
//! }
//!
//! fn main() {
//!     let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
//!     // start the server. This block indefinitely
//!     serve(&addr, &ExampleServer);
//! }
//! ```
use std::io;
use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;

use codec::Codec;
use message::{Message, Response};

use futures::{Async, BoxFuture, Future, Poll, Sink, Stream};
use rmpv::Value;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::Core;
use tokio_io::AsyncRead;
use tokio_io::codec::Framed;

/// The `Service` trait defines how the server handles the requests and notifications it receives.
pub trait Service {
    type Error: Error;
    type T: Into<Value>;
    type E: Into<Value>;

    /// Handle a `MessagePack-RPC` request
    fn handle_request(
        &mut self,
        method: &str,
        params: &[Value],
    ) -> BoxFuture<Result<Self::T, Self::E>, Self::Error>;

    /// Handle a `MessagePack-RPC` notification
    fn handle_notification(&mut self, method: &str, params: &[Value])
        -> BoxFuture<(), Self::Error>;
}

/// Since a new `Service` is created for each client, it is necessary to have a builder type that
/// implements the `ServiceBuilder` trait.
pub trait ServiceBuilder {
    type Service: Service + 'static;

    fn build(&self) -> Self::Service;
}

struct Server<S: Service> {
    service: S,
    done: bool,
    stream: Framed<TcpStream, Codec>,
    request_tasks: HashMap<u32, BoxFuture<Result<S::T, S::E>, S::Error>>,
    notification_tasks: Vec<BoxFuture<(), S::Error>>,
}

impl<S> Server<S>
where
    S: Service + 'static,
{
    fn new(service: S, tcp_stream: TcpStream) -> Self {
        Server {
            service: service,
            done: false,
            stream: tcp_stream.framed(Codec),
            request_tasks: HashMap::new(),
            notification_tasks: Vec::new(),
        }
    }

    fn handle_msg(&mut self, msg: Message) {
        match msg {
            Message::Request(request) => {
                let method = request.method.as_str();
                let params = request.params;
                let response = self.service.handle_request(method, &params);
                self.request_tasks.insert(request.id, response);
            }
            Message::Notification(notification) => {
                let method = notification.method.as_str();
                let params = notification.params;
                let outcome = self.service.handle_notification(method, &params);
                self.notification_tasks.push(outcome);
            }
            Message::Response(_) => {
                return;
            }
        }
    }

    fn process_notifications(&mut self) {
        let mut done = vec![];
        for (idx, task) in self.notification_tasks.iter_mut().enumerate() {
            match task.poll().unwrap() {
                Async::Ready(_) => done.push(idx),
                Async::NotReady => continue,
            }
        }
        for idx in done.iter().rev() {
            self.notification_tasks.remove(*idx);
        }
    }

    fn process_requests(&mut self) {
        let mut done = vec![];
        for (id, task) in &mut self.request_tasks {
            match task.poll().unwrap() {
                Async::Ready(response) => {
                    let msg = Message::Response(Response {
                        id: *id,
                        result: response.map(|v| v.into()).map_err(|e| e.into()),
                    });
                    done.push(*id);
                    if !self.stream.start_send(msg).unwrap().is_ready() {
                        panic!("the sink is full")
                    }
                }
                Async::NotReady => continue,
            }
        }

        for idx in done.iter_mut().rev() {
            let _ = self.request_tasks.remove(idx);
        }
    }

    fn flush(&mut self) {
        if self.stream.poll_complete().unwrap().is_ready() {
            self.done = true;
        } else {
            self.done = false;
        }
    }
}

impl<S> Future for Server<S>
where
    S: Service + 'static,
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.stream.poll().unwrap() {
                Async::Ready(Some(msg)) => self.handle_msg(msg),
                Async::Ready(None) => {
                    return Ok(Async::Ready(()));
                }
                Async::NotReady => break,
            }
        }
        self.process_notifications();
        self.process_requests();
        self.flush();
        Ok(Async::NotReady)
    }
}

/// Create a tokio event loop and run the given `Service` on it.
pub fn serve<B: ServiceBuilder>(address: &SocketAddr, service_builder: &B) {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let listener = TcpListener::bind(address, &handle).unwrap();
    core.run(listener.incoming().for_each(|(stream, _address)| {
        let service = service_builder.build();
        let proto = Server::new(service, stream);
        handle.spawn(proto.map_err(|_| ()));
        Ok(())
    })).unwrap()
}
