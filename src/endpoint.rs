use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::io;

use futures::{Async, AsyncSink, Future, Poll, Sink, StartSend, Stream};
use futures::sync::{mpsc, oneshot};
use tokio_io::codec::Framed;
use tokio_io::{AsyncRead, AsyncWrite};
use rmpv::Value;

use message::{Message, Notification, Request};
use message::Response as MsgPackResponse;
use codec::Codec;

/// The `Service` trait defines how a `MessagePack-RPC` server handles requests and notifications.
pub trait Service {
    type Error: Error;
    type T: Into<Value>;
    type E: Into<Value>;

    /// Handle a `MessagePack-RPC` request.
    fn handle_request(
        &mut self,
        method: &str,
        params: &[Value],
    ) -> Box<Future<Item = Result<Self::T, Self::E>, Error = Self::Error>>;

    /// Handle a `MessagePack-RPC` notification.
    fn handle_notification(
        &mut self,
        method: &str,
        params: &[Value],
    ) -> Box<Future<Item = (), Error = Self::Error>>;
}

struct Server<S: Service> {
    service: S,
    request_tasks: HashMap<u32, Box<Future<Item = Result<S::T, S::E>, Error = S::Error>>>,
    notification_tasks: Vec<Box<Future<Item = (), Error = S::Error>>>,
}

impl<S: Service> Server<S> {
    fn new(service: S) -> Self {
        Server {
            service: service,
            request_tasks: HashMap::new(),
            notification_tasks: Vec::new(),
        }
    }

    fn poll_notification_tasks(&mut self) {
        trace!("Polling pending notification tasks");
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

    fn poll_request_tasks<T: AsyncRead + AsyncWrite>(&mut self, stream: &mut Transport<T>) {
        trace!("Polling pending requests");
        let mut done = vec![];
        for (id, task) in &mut self.request_tasks {
            match task.poll().unwrap() {
                Async::Ready(response) => {
                    let msg = Message::Response(MsgPackResponse {
                        id: *id,
                        result: response.map(|v| v.into()).map_err(|e| e.into()),
                    });
                    done.push(*id);
                    stream.send(msg);
                }
                Async::NotReady => continue,
            }
        }

        for idx in done.iter_mut().rev() {
            let _ = self.request_tasks.remove(idx);
        }
    }

    fn process_request(&mut self, request: Request) {
        let method = request.method.as_str();
        let params = request.params;
        let response = self.service.handle_request(method, &params);
        self.request_tasks.insert(request.id, response);
    }

    fn process_notification(&mut self, notification: Notification) {
        let method = notification.method.as_str();
        let params = notification.params;
        let task = self.service.handle_notification(method, &params);
        self.notification_tasks.push(task);
    }
}

type ResponseTx = oneshot::Sender<Result<Value, Value>>;
/// Future response to a request. It resolved once the response is available.
pub struct Response(oneshot::Receiver<Result<Value, Value>>);

type AckTx = oneshot::Sender<()>;

/// A future that resolves when a notification has been effictively sent to the server. It does not
/// guarantees that the server receives it, just that it has been sent.
pub struct Ack(oneshot::Receiver<()>);

type RequestTx = mpsc::UnboundedSender<(Request, ResponseTx)>;
type RequestRx = mpsc::UnboundedReceiver<(Request, ResponseTx)>;

type NotificationTx = mpsc::UnboundedSender<(Notification, AckTx)>;
type NotificationRx = mpsc::UnboundedReceiver<(Notification, AckTx)>;

impl Future for Response {
    type Item = Result<Value, Value>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll().map_err(|_| ())
    }
}

impl Future for Ack {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll().map_err(|_| ())
    }
}

struct InnerClient {
    shutting_down: bool,
    request_id: u32,
    requests_rx: RequestRx,
    notifications_rx: NotificationRx,
    pending_requests: HashMap<u32, ResponseTx>,
    pending_notifications: Vec<AckTx>,
}

impl InnerClient {
    fn new() -> (Self, Client) {
        let (requests_tx, requests_rx) = mpsc::unbounded();
        let (notifications_tx, notifications_rx) = mpsc::unbounded();

        let client_proxy = Client::new(requests_tx, notifications_tx);

        let client = InnerClient {
            shutting_down: false,
            request_id: 0,
            requests_rx: requests_rx,
            notifications_rx: notifications_rx,
            pending_requests: HashMap::new(),
            pending_notifications: Vec::new(),
        };

        (client, client_proxy)
    }

    fn shutdown(&mut self) {
        trace!("Shutting down inner client");
        self.shutting_down = true;
    }

    fn is_shutting_down(&self) -> bool {
        self.shutting_down
    }

    fn process_notifications<T: AsyncRead + AsyncWrite>(&mut self, stream: &mut Transport<T>) {
        trace!("Polling client notifications channel");
        loop {
            match self.notifications_rx.poll() {
                Ok(Async::Ready(Some((notification, ack_sender)))) => {
                    trace!("Got notification from client.");
                    stream.send(Message::Notification(notification));
                    self.pending_notifications.push(ack_sender);
                }
                Ok(Async::NotReady) => {
                    trace!("No new notification from client");
                    break;
                }
                Ok(Async::Ready(None)) => {
                    trace!("Client closed the notifications channel.");
                    self.shutdown();
                    break;
                }
                Err(()) => {
                    // I have no idea how this should be handled.
                    // The documentation does not tell what may trigger an error.
                    panic!("An error occured while polling the notifications channel.")
                }
            }
        }
    }

    fn process_requests<T: AsyncRead + AsyncWrite>(&mut self, stream: &mut Transport<T>) {
        trace!("Polling client requests channel");
        loop {
            match self.requests_rx.poll() {
                Ok(Async::Ready(Some((mut request, response_sender)))) => {
                    self.request_id += 1;
                    trace!("Got request from client: {:?}", request);
                    request.id = self.request_id;
                    stream.send(Message::Request(request));
                    self.pending_requests
                        .insert(self.request_id, response_sender);
                }
                Ok(Async::Ready(None)) => {
                    trace!("Client closed the requests channel.");
                    self.shutdown();
                    break;
                }
                Ok(Async::NotReady) => {
                    trace!("No new request from client");
                    break;
                }
                Err(()) => {
                    // I have no idea how this should be handled.
                    // The documentation does not tell what may trigger an error.
                    panic!("An error occured while polling the requests channel");
                }
            }
        }
    }

    fn process_response(&mut self, response: MsgPackResponse) {
        if self.is_shutting_down() {
            return;
        }
        if let Some(response_tx) = self.pending_requests.remove(&response.id) {
            trace!("Forwarding response to the client.");
            if let Err(e) = response_tx.send(response.result) {
                warn!("Failed to send response to client: {:?}", e);
            }
        } else {
            warn!("no pending request found for response {}", &response.id);
        }
    }

    fn acknowledge_notifications(&mut self) {
        for chan in self.pending_notifications.drain(..) {
            trace!("Acknowledging notification.");
            if let Err(e) = chan.send(()) {
                warn!("Failed to send ack to client: {:?}", e);
            }
        }
    }
}

pub struct Endpoint<S: Service, T: AsyncRead + AsyncWrite> {
    stream: RefCell<Transport<T>>,
    client: Option<RefCell<InnerClient>>,
    server: Option<RefCell<Server<S>>>,
}

struct Transport<T: AsyncRead + AsyncWrite>(Framed<T, Codec>);

impl<T> Transport<T>
where
    T: AsyncRead + AsyncWrite,
{
    fn send(&mut self, message: Message) {
        trace!("Sending {:?}", message);
        match self.start_send(message) {
            Ok(AsyncSink::Ready) => return,
            // FIXME: there should probably be a retry mechanism.
            Ok(AsyncSink::NotReady(_message)) => panic!("The sink is full."),
            Err(e) => panic!("An error occured while trying to send message: {:?}", e),
        }
    }
}

impl<T> Stream for Transport<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Item = Message;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.0.poll()
    }
}

impl<T> Sink for Transport<T>
where
    T: AsyncRead + AsyncWrite,
{
    type SinkItem = Message;
    type SinkError = io::Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        self.0.start_send(item)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        self.0.poll_complete()
    }
}

impl<S, T> Endpoint<S, T>
where
    S: Service,
    T: AsyncRead + AsyncWrite,
{
    pub fn new(stream: T) -> Self {
        Endpoint {
            stream: RefCell::new(Transport(stream.framed(Codec))),
            client: None,
            server: None,
        }
    }

    pub fn set_server(&mut self, service: S) {
        self.server = Some(RefCell::new(Server::new(service)));
    }

    pub fn set_client(&mut self) -> Client {
        let (client, client_proxy) = InnerClient::new();
        self.client = Some(RefCell::new(client));
        client_proxy
    }

    fn handle_message(&mut self, msg: Message) {
        trace!("Received {:?}", msg);
        match msg {
            Message::Request(request) => if let Some(ref mut server) = self.server {
                server.get_mut().process_request(request);
            } else {
                trace!("This endpoint does not handle requests. Ignoring it.");
            },
            Message::Notification(notification) => if let Some(ref mut server) = self.server {
                server.get_mut().process_notification(notification);
            } else {
                trace!("This endpoint does not handle notifications. Ignoring it.");
            },
            Message::Response(response) => if let Some(ref mut client) = self.client {
                client.get_mut().process_response(response);
            } else {
                trace!("This endpoint does not handle responses. Ignoring it.");
            },
        }
    }

    fn flush(&mut self) {
        trace!("Flushing stream");
        match self.stream.get_mut().poll_complete() {
            Ok(Async::Ready(())) => if let Some(ref mut client) = self.client {
                client.get_mut().acknowledge_notifications();
            },
            Ok(Async::NotReady) => return,
            Err(e) => panic!("Failed to flush the sink: {:?}", e),
        }
    }
}

impl<S, T: AsyncRead + AsyncWrite> Future for Endpoint<S, T>
where
    S: Service,
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        trace!("Polling stream.");
        loop {
            match self.stream.get_mut().poll().unwrap_or_else(|e| {
                warn!("Error on stream: {}", e);
                // Drop this connection on error
                Async::Ready(None)
            }) {
                Async::Ready(Some(msg)) => self.handle_message(msg),
                Async::Ready(None) => {
                    trace!("Stream closed by remote peer.");
                    // FIXME: not sure if we should still continue sending responses here. Is it
                    // possible that the client closed the stream only one way and is still waiting
                    // for response? Not for TCP at least, but maybe for other transport types?
                    return Ok(Async::Ready(()));
                }
                Async::NotReady => {
                    trace!("No new message in the stream");
                    break;
                }
            }
        }

        if let Some(ref mut server) = self.server {
            let server = server.get_mut();
            server.poll_request_tasks(self.stream.get_mut());
            server.poll_notification_tasks();
        }

        let mut client_shutdown: bool = false;
        if let Some(ref mut client) = self.client {
            let client = client.get_mut();
            let stream = self.stream.get_mut();
            client.process_requests(stream);
            client.process_notifications(stream);
            if client.is_shutting_down() {
                trace!("Client shut down, exiting");
                client_shutdown = true;
            }
        }
        if client_shutdown {
            self.client = None;
        }

        self.flush();

        trace!("notifying the reactor that we're not done yet");
        Ok(Async::NotReady)
    }
}

/// A `Service` builder. This trait must be implemented for servers.
pub trait ServiceBuilder {
    type Service: Service + 'static;

    fn build(&self, client: Client) -> Self::Service;
}

/// A client that sends requests and notifications to a remote MessagePack-RPC server.
#[derive(Clone)]
pub struct Client {
    requests_tx: RequestTx,
    notifications_tx: NotificationTx,
}

impl Client {
    fn new(requests_tx: RequestTx, notifications_tx: NotificationTx) -> Self {
        Client {
            requests_tx: requests_tx,
            notifications_tx: notifications_tx,
        }
    }
    /// Send a `MessagePack-RPC` request
    pub fn request(&self, method: &str, params: &[Value]) -> Response {
        trace!("New request (method={}, params={:?})", method, params);
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
        let _ = mpsc::UnboundedSender::unbounded_send(&self.requests_tx, (request, tx));
        Response(rx)
    }

    /// Send a `MessagePack-RPC` notification
    pub fn notify(&self, method: &str, params: &[Value]) -> Ack {
        trace!("New notification (method={}, params={:?})", method, params);
        let notification = Notification {
            method: method.to_owned(),
            params: Vec::from(params),
        };
        let (tx, rx) = oneshot::channel();
        let _ = mpsc::UnboundedSender::unbounded_send(&self.notifications_tx, (notification, tx));
        Ack(rx)
    }
}

impl Future for Client {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready(()))
    }
}
