use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::io;
use std::net::SocketAddr;

use futures::{Async, AsyncSink, BoxFuture, Canceled, Future, Poll, Sink, Stream};
use futures::sync::{mpsc, oneshot};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::{Core, Handle};
use tokio_io::codec::Framed;
use tokio_io::AsyncRead;
use rmpv::Value;

use message::{Message, Notification, Request};
use message::Response as MsgPackResponse;
use codec::Codec;

/// The `Service` trait defines how a `MessagePack-RPC` server handles requests and notifications.
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

struct Server<S: Service> {
    service: S,
    request_tasks: HashMap<u32, BoxFuture<Result<S::T, S::E>, S::Error>>,
    notification_tasks: Vec<BoxFuture<(), S::Error>>,
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

    fn poll_request_tasks(&mut self, stream: &mut Transport) {
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
                    // self.send(msg);
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

    fn process_notifications(&mut self, stream: &mut Transport) {
        loop {
            match self.notifications_rx.poll() {
                Ok(Async::Ready(Some((notification, ack_sender)))) => {
                    trace!("Got notification from client.");
                    stream.send(Message::Notification(notification));
                    self.pending_notifications.push(ack_sender);
                }
                Ok(Async::NotReady) => return,
                Ok(Async::Ready(None)) => {
                    trace!("Client closed the notifications channel.");
                    self.shutdown();
                    return;
                }
                Err(()) => {
                    // I have no idea how this should be handled.
                    // The documentation does not tell what may trigger an error.
                    panic!("An error occured while polling the notifications channel.")
                }
            }
        }
    }

    fn process_requests(&mut self, stream: &mut Transport) {
        loop {
            match self.requests_rx.poll() {
                Ok(Async::Ready(Some((mut request, response_sender)))) => {
                    self.request_id += 1;
                    trace!("Got request from client.");
                    request.id = self.request_id;
                    stream.send(Message::Request(request));
                    self.pending_requests
                        .insert(self.request_id, response_sender);
                }
                Ok(Async::NotReady) => return,
                Ok(Async::Ready(None)) => {
                    trace!("Client closed the requests channel.");
                    self.shutdown();
                    return;
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

struct Endpoint<S: Service> {
    stream: RefCell<Transport>,
    client: Option<RefCell<InnerClient>>,
    server: Option<RefCell<Server<S>>>,
}

struct Transport(Framed<TcpStream, Codec>);

impl Transport {
    fn send(&mut self, message: Message) {
        trace!(">>> {:?}", message);
        match self.0.start_send(message) {
            Ok(AsyncSink::Ready) => return,
            // FIXME: there should probably be a retry mechanism.
            Ok(AsyncSink::NotReady(_message)) => panic!("The sink is full."),
            Err(e) => panic!("An error occured while trying to send message: {:?}", e),
        }
    }

    fn poll_complete(&mut self) -> Result<Async<()>, io::Error> {
        self.0.poll_complete()
    }

    fn poll(&mut self) -> Result<Async<Option<Message>>, io::Error> {
        self.0.poll()
    }
}

impl<S: Service> Endpoint<S> {
    fn new(stream: TcpStream) -> Self {
        Endpoint {
            stream: RefCell::new(Transport(stream.framed(Codec))),
            client: None,
            server: None,
        }
    }

    fn set_server(&mut self, service: S) {
        self.server = Some(RefCell::new(Server::new(service)));
    }

    fn set_client(&mut self) -> Client {
        let (client, client_proxy) = InnerClient::new();
        self.client = Some(RefCell::new(client));
        client_proxy
    }

    fn handle_message(&mut self, msg: Message) {
        trace!("<<< {:?}", msg);
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
        match self.stream.get_mut().poll_complete() {
            Ok(Async::Ready(())) => {
                if let Some(ref mut client) = self.client {
                    client.get_mut().acknowledge_notifications();
                }
            }
            Ok(Async::NotReady) => return,
            Err(e) => panic!("Failed to flush the sink: {:?}", e),
        }
    }
}

impl<S: Service> Future for Endpoint<S> {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        trace!("Processing received messages.");
        loop {
            match self.stream.get_mut().poll().unwrap() {
                Async::Ready(Some(msg)) => self.handle_message(msg),
                Async::Ready(None) => {
                    trace!("Stream closed by remote peer.");
                    // FIXME: not sure if we should still continue sending responses here. Is it
                    // possible that the client closed the stream only one way and is still waiting
                    // for response? Not for TCP at least, but maybe for other transport types?
                    return Ok(Async::Ready(()));
                }
                Async::NotReady => break,
            }
        }
        trace!("Done processing received messages.");

        if let Some(ref mut server) = self.server {
            let server = server.get_mut();
            server.poll_request_tasks(self.stream.get_mut());
            server.poll_notification_tasks();
        }

        if let Some(ref mut client) = self.client {
            let client = client.get_mut();
            let stream = self.stream.get_mut();
            client.process_requests(stream);
            client.process_notifications(stream);
            if client.is_shutting_down() {
                trace!("Client shut down, exiting");
                return Ok(Async::Ready(()));
            }
        }

        self.flush();
        Ok(Async::NotReady)
    }
}

/// A `Service` builder. This trait must be implemented for servers.
pub trait ServiceBuilder {
    type Service: Service + 'static;

    fn build(&self) -> Self::Service;
}

/// Start a `MessagePack-RPC` server.
pub fn serve<B: ServiceBuilder>(address: &SocketAddr, service_builder: &B) {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let listener = TcpListener::bind(address, &handle).unwrap();
    core.run(listener.incoming().for_each(|(stream, _address)| {
        let mut endpoint = Endpoint::new(stream);
        endpoint.set_server(service_builder.build());
        handle.spawn(endpoint.map_err(|_| ()));
        Ok(())
    })).unwrap()
}

/// A client that sends requests and notifications to a remote MessagePack-RPC server.
pub struct Client {
    requests_tx: RequestTx,
    notifications_tx: NotificationTx,
}

impl Clone for Client {
    fn clone(&self) -> Self {
        Client {
            requests_tx: self.requests_tx.clone(),
            notifications_tx: self.notifications_tx.clone(),
        }
    }
}

impl Client {
    pub fn new(requests_tx: RequestTx, notifications_tx: NotificationTx) -> Self {
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
        let _ = mpsc::UnboundedSender::send(&self.requests_tx, (request, tx));
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
        let _ = mpsc::UnboundedSender::send(&self.notifications_tx, (notification, tx));
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

/// A `Connector` is used to initiate a connection with a remote `MessagePack-RPC` endpoint.
/// Establishing the connection consumes the `Connector` and gives a
/// [`Connection`](struct.Connection.html).
///
/// A `Connector` should be used only if you need to create a `MessagePack-RPC` endpoint that
/// behaves both like a client (_i.e._ sends requests and notifications to the remote endpoint) and
/// like a server (_i.e._ handles incoming `MessagePack-RPC` requests and notifications). If you
/// need a regular client that only sends `MessagePack-RPC` requests and notifications, use
/// [`DefaultConnector`](struct.DefaultConnector.html).
pub struct Connector<'a, 'b, S> {
    service: Option<S>,
    address: &'a SocketAddr,
    handle: &'b Handle,
}

impl<'a, 'b, S: Service + 'static> Connector<'a, 'b, S> {
    /// Create a new `Connector`. `address` is the address of the remote `MessagePack-RPC` server.
    pub fn new(address: &'a SocketAddr, handle: &'b Handle) -> Self {
        Connector {
            service: None,
            address: address,
            handle: handle,
        }
    }

    /// Make the client able to handle incoming requests and notification using the given service.
    /// Once the connection is established, the client will act as a server and answer requests and
    /// notifications in background, using this service.
    pub fn set_service(&mut self, service: S) -> &mut Self {
        self.service = Some(service);
        self
    }

    /// Connect to the remote `MessagePack-RPC` endpoint. This consumes the `Connector`.
    pub fn connect(mut self) -> Connection {
        trace!("Trying to connect to {}.", self.address);

        let (client_tx, client_rx) = oneshot::channel();
        let (error_tx, error_rx) = oneshot::channel();

        let connection = Connection {
            client_rx: client_rx,
            error_rx: error_rx,
        };

        let service = self.service.take();
        let client_proxy = TcpStream::connect(self.address, self.handle)
            .and_then(move |stream| {
                trace!("Connection established.");
                let mut endpoint = Endpoint::new(stream);
                if let Some(service) = service {
                    endpoint.set_server(service);
                }
                let client_proxy = endpoint.set_client();
                if client_tx.send(client_proxy).is_err() {
                    panic!("Failed to send client to connection");
                }
                endpoint
            })
            .or_else(|e| {
                error!("Connection failed: {:?}.", e);
                if let Err(e) = error_tx.send(e) {
                    panic!("Failed to send client to connection: {:?}", e);
                }
                Err(())
            });

        trace!("Spawning Endpoint and returning Connection");
        self.handle.spawn(client_proxy);
        connection
    }
}

struct NoService;

impl Service for NoService {
    type Error = io::Error;
    type T = String;
    type E = String;

    fn handle_request(
        &mut self,
        _method: &str,
        _params: &[Value],
    ) -> BoxFuture<Result<Self::T, Self::E>, Self::Error> {
        unreachable!();
    }

    /// Handle a `MessagePack-RPC` notification
    fn handle_notification(
        &mut self,
        _method: &str,
        _params: &[Value],
    ) -> BoxFuture<(), Self::Error> {
        unreachable!();
    }
}

/// A `Connector` for `MessagePack-RPC` clients. See also [`Connector`](struct.Connector.html).
pub struct DefaultConnector<'a, 'b>(Connector<'a, 'b, NoService>);

impl<'a, 'b> DefaultConnector<'a, 'b> {
    /// Create a new `DefaultConnector`.
    pub fn new(address: &'a SocketAddr, handle: &'b Handle) -> Self {
        DefaultConnector(Connector::<'a, 'b, NoService>::new(address, handle))
    }

    /// Connect to the remote `MessagePack-RPC` server.
    pub fn connect(self) -> Connection {
        self.0.connect()
    }
}

/// A future that returns a `MessagePack-RPC` endpoint when it completes successfully.
pub struct Connection {
    client_rx: oneshot::Receiver<Client>,
    error_rx: oneshot::Receiver<io::Error>,
}

impl Connection {
    fn poll_error(&mut self) -> Option<io::Error> {
        match self.error_rx.poll() {
            Ok(Async::Ready(e)) => Some(e),
            Ok(Async::NotReady) => None,
            // an error means that the remote end of the channel got dropped
            Err(Canceled) => panic!("errors while polling error channel (client dropped?)"),
        }
    }

    fn poll_client(&mut self) -> Option<Client> {
        match self.client_rx.poll() {
            Ok(Async::Ready(client)) => Some(client),
            Ok(Async::NotReady) => None,
            // an error means that the remote end of the channel got dropped
            Err(Canceled) => panic!("errors while polling client channel (client dropped?)"),
        }
    }
}

impl Future for Connection {
    type Item = Client;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(client) = self.poll_client() {
            trace!("Connection: terminating successfully and returning Client");
            Ok(Async::Ready(client))
        } else if let Some(e) = self.poll_error() {
            trace!("Connection: terminating with an error {}", e);
            Err(e)
        } else {
            Ok(Async::NotReady)
        }
    }
}
