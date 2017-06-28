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

impl Clone for ClientProxy {
    fn clone(&self) -> Self {
        ClientProxy {
            requests_tx: self.requests_tx.clone(),
            notifications_tx: self.notifications_tx.clone(),
        }
    }
}

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

pub struct ClientProxy {
    requests_tx: mpsc::UnboundedSender<(Request, oneshot::Sender<Result<Value, Value>>)>,
    notifications_tx: mpsc::UnboundedSender<(Notification, oneshot::Sender<()>)>,
}

struct Client {
    stream: Framed<TcpStream, Codec>,
    request_id: u32,
    shutdown: bool,

    requests_rx: mpsc::UnboundedReceiver<(Request, oneshot::Sender<Result<Value, Value>>)>,
    notifications_rx: mpsc::UnboundedReceiver<(Notification, oneshot::Sender<()>)>,

    pending_requests: HashMap<u32, oneshot::Sender<Result<Value, Value>>>,
    pending_notifications: Vec<oneshot::Sender<()>>,
}

impl ClientProxy {
    pub fn request(&mut self, method: &str, params: &[Value]) -> Response {
        let request = Request {
            id: 0,
            method: method.to_owned(),
            params: Vec::from(params),
        };
        let (tx, rx) = oneshot::channel();
        // If send returns an Err, its because the other side has been dropped. By ignoring it, we
        // are just dropping the `tx`, which will mean the rx will return Canceled when polled. In
        // turn, that is translated into a BrokenPipe, which conveys the proper error.
        let _ = mpsc::UnboundedSender::send(&self.requests_tx, (request, tx));
        Response { inner: rx }
    }

    pub fn notify(&mut self, method: &str, params: &[Value]) -> Ack {
        let notification = Notification {
            method: method.to_owned(),
            params: Vec::from(params),
        };
        let (tx, rx) = oneshot::channel();
        let _ = mpsc::UnboundedSender::send(&self.notifications_tx, (notification, tx));
        Ack { inner: rx }
    }

    pub fn connect(addr: &SocketAddr, handle: &Handle) -> Connection {
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
                let (requests_tx, requests_rx) = mpsc::unbounded();
                let (notifications_tx, notifications_rx) = mpsc::unbounded();

                let client_proxy = ClientProxy {
                    requests_tx: requests_tx,
                    notifications_tx: notifications_tx,
                };

                if client_proxy_tx.send(client_proxy).is_err() {
                    panic!("Failed to send client proxy to connection");
                }

                Client {
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
                if let Err(e) = error_tx.send(e) {
                    panic!("Failed to send client proxy to connection: {:?}", e);
                }
                Err(())
            });

        // Start the `Client`
        handle.spawn(client);

        // Return the `Connection`. It's a future that resolves when it receives the `ClientProxy`.
        connection
    }
}

pub struct Connection {
    client_proxy_rx: oneshot::Receiver<ClientProxy>,
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
    fn poll_client_proxy(&mut self) -> Option<ClientProxy> {
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
    type Item = ClientProxy;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(client_proxy) = self.poll_client_proxy() {
            Ok(Async::Ready(client_proxy))
        } else if let Some(e) = self.poll_error() {
            Err(e)
        } else if self.client_proxy_chan_cancelled && self.error_chan_cancelled {
            panic!("Failed to receive outcome of the connection");
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl Client {
    fn handle_msg(&mut self, msg: Message) {
        match msg {
            Message::Request(_) |
            Message::Notification(_) => (),
            Message::Response(response) => {
                if let Some(response_sender) = self.pending_requests.remove(&response.id) {
                    response_sender.send(response.result).unwrap();
                }
            }
        }
    }

    fn process_notifications(&mut self) {
        loop {
            match self.notifications_rx.poll().unwrap() {
                Async::Ready(Some((notification, ack_sender))) => {
                    let send_task = self.stream
                        .start_send(Message::Notification(notification))
                        .unwrap();
                    if !send_task.is_ready() {
                        panic!("the sink is full")
                    }
                    self.pending_notifications.push(ack_sender);
                }
                Async::Ready(None) => {
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
                    request.id = self.request_id;
                    let send_task = self.stream.start_send(Message::Request(request)).unwrap();
                    if !send_task.is_ready() {
                        panic!("the sink is full")
                    }
                    self.pending_requests
                        .insert(self.request_id, response_sender);
                }
                Async::Ready(None) => {
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
                ack_sender.send(()).unwrap();
            }
        }
    }
}


impl Future for Client {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.stream.poll().unwrap() {
                Async::Ready(Some(msg)) => self.handle_msg(msg),
                Async::Ready(None) => return Ok(Async::Ready(())),
                Async::NotReady => break,
            }
        }
        if self.shutdown {
            if self.pending_requests.is_empty() {
                Ok(Async::Ready(()))
            } else {
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

impl Future for ClientProxy {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready(()))
    }
}
