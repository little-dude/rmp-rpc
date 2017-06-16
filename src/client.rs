use tokio_core::net::TcpStream;
use tokio_core::reactor::Handle;
use tokio_io::codec::Framed;

use std::io;
use std::net::SocketAddr;
use message::{Request, Notification, Message};
use rmpv::Value;
use futures::{future, Async, Poll, Future, BoxFuture, Stream, Sink};
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

    pub fn connect(addr: &SocketAddr, handle: &Handle) -> BoxFuture<ClientProxy, ()> {
        let (requests_tx, requests_rx) = mpsc::unbounded();
        let (notifications_tx, notifications_rx) = mpsc::unbounded();

        let client_proxy = ClientProxy {
            requests_tx: requests_tx,
            notifications_tx: notifications_tx,
        };

        let client = TcpStream::connect(addr, handle)
            .and_then(|stream| {
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
            .map_err(|_| ());

        handle.spawn(client);
        Box::new(future::ok(client_proxy))
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
