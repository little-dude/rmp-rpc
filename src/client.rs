//! This module provides a `MessagePack-RPC` asynchronous client.
//!
//! # Examples
//!
//! ```rust,no_run
//! extern crate futures;
//! extern crate rmp_rpc;
//! extern crate tokio_core;
//!
//! use std::net::SocketAddr;
//!
//! use futures::Future;
//! use rmp_rpc::{Value, Integer};
//! use rmp_rpc::client::Client;
//! use tokio_core::reactor::Core;
//!
//! fn main() {
//!    // Create the tokio event loop
//!    let mut core = Core::new().unwrap();
//!    let handle = core.handle();
//!
//!    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
//!
//!    let task =
//!        // Connect to the server
//!        Client::connect(&addr, &handle)
//!        .or_else(|e| {
//!            println!("Connection to server failed: {}", e);
//!            Err(())
//!        })
//!        .and_then(|client| {
//!            // Send a msgpack-rpc notification, with method "ping" and no argument
//!            client.notify("ping", &[]).and_then(|_| {
//!                // Return the client, so that we can reuse it
//!                Ok(client)
//!            })
//!        })
//!        .and_then(|client| {
//!            // Send a msgpack-rpc request, with method "add" and two arguments
//!            let args = vec![Value::Integer(Integer::from(3)),
//!                            Value::Integer(Integer::from(4))];
//!            client.request("add", &args).and_then(|response| {
//!                // Handle the response [...]
//!                Ok(())
//!            })
//!        });
//!    core.run(task).unwrap();
//! }
//! ```

use tokio_core::net::TcpStream;
use tokio_core::reactor::Handle;
use tokio_io::codec::Framed;

use std::io;
use std::net::SocketAddr;
use message::{Request, Notification, Message};
use rmpv::Value;
use futures::{Async, Poll, Future, Stream, Sink};
use futures::sync::{mpsc, oneshot};
use tokio_io::AsyncRead;
use codec::Codec;
use std::collections::HashMap;

/// A future response.to a request.
pub struct Response {
    inner: oneshot::Receiver<Result<Value, Value>>,
}

impl Future for Response {
    type Item = Result<Value, Value>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll().map_err(|_| ())
    }
}

/// A future that signals that a notifications has been sent to the server.
pub struct Ack {
    inner: oneshot::Receiver<()>,
}

impl Future for Ack {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll().map_err(|_| ())
    }
}

/// A client used to send requests on notifications to a `MessagePack-RPC` server.
///
pub struct Client {
    requests_tx: mpsc::UnboundedSender<(Request, oneshot::Sender<Result<Value, Value>>)>,
    notifications_tx: mpsc::UnboundedSender<(Notification, oneshot::Sender<()>)>,
}

impl Clone for Client {
    fn clone(&self) -> Self {
        Client {
            requests_tx: self.requests_tx.clone(),
            notifications_tx: self.notifications_tx.clone(),
        }
    }
}

struct Endpoint {
    stream: Framed<TcpStream, Codec>,
    request_id: u32,
    shutdown: bool,

    requests_rx: mpsc::UnboundedReceiver<(Request, oneshot::Sender<Result<Value, Value>>)>,
    notifications_rx: mpsc::UnboundedReceiver<(Notification, oneshot::Sender<()>)>,

    pending_requests: HashMap<u32, oneshot::Sender<Result<Value, Value>>>,
    pending_notifications: Vec<oneshot::Sender<()>>,
}

impl Client {
    /// Send a `MessagePack-RPC` request
    pub fn request(&self, method: &str, params: &[Value]) -> Response {
        trace!(
            "Client: new request (method={}, params={:?})",
            method,
            params
            );
        let request = Request {
            id: 0,
            method: method.to_owned(),
            params: Vec::from(params),
        };
        let (tx, rx) = oneshot::channel();
        // If send returns an Err, its because the other side has been dropped. By ignoring it,
        // we are just dropping the `tx`, which will mean the rx will return Canceled when
        // polled. In turn, that is translated into a BrokenPipe, which conveys the proper
        // error.
        let _ = mpsc::UnboundedSender::send(&self.requests_tx, (request, tx));
        Response { inner: rx }
    }

    /// Send a `MessagePack-RPC` notification
    pub fn notify(&self, method: &str, params: &[Value]) -> Ack {
        trace!(
            "Client: new notification (method={}, params={:?})",
            method,
            params
            );
        let notification = Notification {
            method: method.to_owned(),
            params: Vec::from(params),
        };
        let (tx, rx) = oneshot::channel();
        let _ = mpsc::UnboundedSender::send(&self.notifications_tx, (notification, tx));
        Ack { inner: rx }
    }

    /// Connect the client to a remote `MessagePack-RPC` server.
    pub fn connect(addr: &SocketAddr, handle: &Handle) -> Connection {
        trace!("Client: trying to connect to {}", addr);
        let (client_proxy_tx, client_proxy_rx) = oneshot::channel();
        let (error_tx, error_rx) = oneshot::channel();

        let connection = Connection {
            client_proxy_rx: client_proxy_rx,
            error_rx: error_rx,
            client_proxy_chan_cancelled: false,
            error_chan_cancelled: false,
        };

        let client = TcpStream::connect(addr, handle)
            .and_then(|stream| {
                trace!("Client: connection established");
                let (requests_tx, requests_rx) = mpsc::unbounded();
                let (notifications_tx, notifications_rx) = mpsc::unbounded();

                let client_proxy = Client {
                    requests_tx: requests_tx,
                    notifications_tx: notifications_tx,
                };

                if client_proxy_tx.send(client_proxy).is_err() {
                    panic!("Failed to send client proxy to connection");
                }

                Endpoint {
                    request_id: 0,
                    shutdown: false,
                    stream: stream.framed(Codec),
                    requests_rx: requests_rx,
                    notifications_rx: notifications_rx,
                    pending_requests: HashMap::new(),
                    pending_notifications: Vec::new(),
                }
            })
        .or_else(|e| {
            error!("Client: connection failed: {}", e);
            if let Err(e) = error_tx.send(e) {
                panic!("Failed to send client proxy to connection: {:?}", e);
            }
            Err(())
        });

        trace!("Spawning Endpoint and returning Connection");
        handle.spawn(client);
        connection
    }
}

/// A future that returns a `Endpoint` when it completes successfully.
pub struct Connection {
    client_proxy_rx: oneshot::Receiver<Client>,
    client_proxy_chan_cancelled: bool,
    error_rx: oneshot::Receiver<io::Error>,
    error_chan_cancelled: bool,
}

impl Connection {
    fn poll_error(&mut self) -> Option<io::Error> {
        if self.error_chan_cancelled {
            return None;
        }
        match self.error_rx.poll() {
            Ok(Async::Ready(e)) => Some(e),
            Ok(Async::NotReady) => None,
            Err(_) => {
                self.error_chan_cancelled = true;
                None
            }
        }
    }
    fn poll_client_proxy(&mut self) -> Option<Client> {
        if self.client_proxy_chan_cancelled {
            return None;
        }
        match self.client_proxy_rx.poll() {
            Ok(Async::Ready(client_proxy)) => Some(client_proxy),
            Ok(Async::NotReady) => None,
            Err(_) => {
                self.client_proxy_chan_cancelled = true;
                None
            }
        }
    }
}

impl Future for Connection {
    type Item = Client;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(client_proxy) = self.poll_client_proxy() {
            trace!("Connection: terminating successfully and returning Client");
            Ok(Async::Ready(client_proxy))
        } else if let Some(e) = self.poll_error() {
            trace!("Connection: terminating with an error {}", e);
            Err(e)
        } else if self.client_proxy_chan_cancelled && self.error_chan_cancelled {
            panic!("Failed to receive outcome of the connection");
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl Endpoint {
    fn handle_msg(&mut self, msg: Message) {
        match msg {
            Message::Request(_) |
                Message::Notification(_) => {
                    trace!("Endpoint: got a request or notification from server. Ignoring it.");
                }
            Message::Response(response) => {
                if let Some(response_sender) = self.pending_requests.remove(&response.id) {
                    trace!(
                        "Endpoint: got a response from server: {:?}, \
                             and found the corresponding pending request.",
                             response
                          );
                    response_sender.send(response.result).unwrap();
                } else {
                    trace!(
                            "Endpoint: got a response from server: {:?}, \
                            but no corresponding pending request. Ignoring it.",
                            response
                        );
                    }
                }
            }
        }

        fn process_notifications(&mut self) {
            loop {
                match self.notifications_rx.poll().unwrap() {
                    Async::Ready(Some((notification, ack_sender))) => {
                        trace!(
                            "Endpoint: received notification from Client. \
                            Forwarding it to the server."
                            );
                        let send_task = self.stream
                            .start_send(Message::Notification(notification))
                            .unwrap();
                        if !send_task.is_ready() {
                            panic!("the sink is full")
                        }
                        self.pending_notifications.push(ack_sender);
                    }
                    Async::Ready(None) => {
                        trace!(
                            "Endpoint: Client closed the remote end of the notifications \
                            channel. Entering shutdown state."
                        );
                        self.shutdown = true;
                        return;
                    }
                    Async::NotReady => return,
                }
            }
        }

        fn process_requests(&mut self) {
            loop {
                match self.requests_rx.poll().unwrap() {
                    Async::Ready(Some((mut request, response_sender))) => {
                        self.request_id += 1;
                        trace!(
                            "Endpoint: received request from Client. \
                            Forwarding it to the server with id {}.",
                            self.request_id
                        );
                        request.id = self.request_id;
                        let send_task = self.stream.start_send(Message::Request(request)).unwrap();
                        if !send_task.is_ready() {
                            panic!("the sink is full")
                        }
                        self.pending_requests
                            .insert(self.request_id, response_sender);
                    }
                    Async::Ready(None) => {
                        trace!(
                            "Endpoint: Client closed the remote end of the requests channel. \
                            Entering shutdown state."
                            );
                        self.shutdown = true;
                        return;
                    }
                    Async::NotReady => return,
                }
            }
        }

        fn flush(&mut self) {
            if self.stream.poll_complete().unwrap().is_ready() {
                for ack_sender in self.pending_notifications.drain(..) {
                    trace!(
                        "Endpoint: letting Client know that pending notification has been sent"
                    );
                    ack_sender.send(()).unwrap();
                }
            }
        }
    }

    impl Future for Endpoint {
        type Item = ();
        type Error = io::Error;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            loop {
                match self.stream.poll().unwrap() {
                    Async::Ready(Some(msg)) => self.handle_msg(msg),
                    Async::Ready(None) => {
                        trace!(
                            "Endpoint: stream with server has been closed. Terminating successfully"
                        );
                        return Ok(Async::Ready(()));
                    }
                    Async::NotReady => break,
                }
            }
            if self.shutdown {
                if self.pending_requests.is_empty() {
                    trace!(
                        "Endpoint: all pending requests have been processed. \
                        Terminating successfully"
                    );
                    Ok(Async::Ready(()))
                } else {
                    trace!(
                        "Endpoint: not all pending requests have been processed. \
                        Waiting before terminating"
                        );
                    Ok(Async::NotReady)
                }
            } else {
                self.process_notifications();
                self.process_requests();
                self.flush();
                Ok(Async::NotReady)
            }
        }
    }

impl Future for Client {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready(()))
    }
}
