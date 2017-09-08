use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::io;
use std::net::SocketAddr;

use futures::{self, Async, AsyncSink, Canceled, Future, Poll, Sink, StartSend, Stream};
use futures::sync::{mpsc, oneshot};
use native_tls::TlsConnector;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::Handle;
use tokio_io::codec::Framed;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_tls::TlsConnectorExt;
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
        match self.notifications_rx.poll() {
            Ok(Async::Ready(Some((notification, ack_sender)))) => {
                trace!("Got notification from client.");
                stream.send(Message::Notification(notification));
                self.pending_notifications.push(ack_sender);
            }
            Ok(Async::NotReady) => trace!("No new notification from client"),
            Ok(Async::Ready(None)) => {
                trace!("Client closed the notifications channel.");
                self.shutdown();
            }
            Err(()) => {
                // I have no idea how this should be handled.
                // The documentation does not tell what may trigger an error.
                panic!("An error occured while polling the notifications channel.")
            }
        }
    }

    fn process_requests<T: AsyncRead + AsyncWrite>(&mut self, stream: &mut Transport<T>) {
        trace!("Polling client requests channel");
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
            }
            Ok(Async::NotReady) => trace!("No new request from client"),
            Err(()) => {
                // I have no idea how this should be handled.
                // The documentation does not tell what may trigger an error.
                panic!("An error occured while polling the requests channel");
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

struct Endpoint<S: Service, T: AsyncRead + AsyncWrite> {
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
    fn new(stream: T) -> Self {
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
        match self.stream.get_mut().poll().unwrap() {
            Async::Ready(Some(msg)) => self.handle_message(msg),
            Async::Ready(None) => {
                trace!("Stream closed by remote peer.");
                // FIXME: not sure if we should still continue sending responses here. Is it
                // possible that the client closed the stream only one way and is still waiting
                // for response? Not for TCP at least, but maybe for other transport types?
                return Ok(Async::Ready(()));
            }
            Async::NotReady => trace!("No new message in the stream"),
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

        trace!("notifying the reactor to reschedule current endpoint for polling");
        // see https://www.coredump.ch/2017/07/05/understanding-the-tokio-reactor-core/
        futures::task::current().notify();
        Ok(Async::NotReady)
    }
}

/// A `Service` builder. This trait must be implemented for servers.
pub trait ServiceBuilder {
    type Service: Service + 'static;

    fn build(&self, client: Client) -> Self::Service;
}

/// Start a `MessagePack-RPC` server.
pub fn serve<B: ServiceBuilder + 'static>(
    address: SocketAddr,
    service_builder: B,
    handle: Handle,
) -> Box<Future<Item = (), Error = ()>> {
    let listener = TcpListener::bind(&address, &handle)
        .unwrap()
        .incoming()
        .for_each(move |(stream, _address)| {
            let mut endpoint = Endpoint::new(stream);
            let client_proxy = endpoint.set_client();
            endpoint.set_server(service_builder.build(client_proxy));
            handle.spawn(endpoint.map_err(|_| ()));
            Ok(())
        })
        .map_err(|_| ());
    Box::new(listener)
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

/// A `Connector` is used to initiate a connection with a remote `MessagePack-RPC` endpoint.
/// Establishing the connection consumes the `Connector` and gives a
/// [`Connection`](struct.Connection.html).
///
/// A `Connector` should be used only if you need to create a `MessagePack-RPC` endpoint that
/// behaves both like a client (_i.e._ sends requests and notifications to the remote endpoint) and
/// like a server (_i.e._ handles incoming `MessagePack-RPC` requests and notifications). If you
/// need a regular client that only sends `MessagePack-RPC` requests and notifications, use
/// [`ClientOnlyConnector`](struct.ClientOnlyConnector.html).
pub struct Connector<'a, 'b, S> {
    service_builder: Option<S>,
    address: &'a SocketAddr,
    handle: &'b Handle,
    tls: bool,
    tls_domain: Option<String>,
}

impl<'a, 'b, S: ServiceBuilder + Sync + Send + 'static> Connector<'a, 'b, S> {
    /// Create a new `Connector`. `address` is the address of the remote `MessagePack-RPC` server.
    pub fn new(address: &'a SocketAddr, handle: &'b Handle) -> Self {
        Connector {
            service_builder: None,
            address: address,
            handle: handle,
            tls: false,
            tls_domain: None,
        }
    }

    /// Enable TLS for this connection. `domain` is the hostname of the remote endpoint so which we
    /// are connecting.
    pub fn set_tls_connector(&mut self, domain: String) -> &mut Self {
        self.tls = true;
        self.tls_domain = Some(domain);
        self
    }

    /// Enable TLS for this connection, but without hostname verification. This is dangerous,
    /// because it means that any server with a valid certificate will be trusted. Hence, it is not
    /// recommended.
    pub fn set_tls_connector_with_hostname_verification_disabled(&mut self) -> &mut Self {
        self.tls = true;
        self.tls_domain = None;
        self
    }

    /// Make the client able to handle incoming requests and notification using the given service.
    /// Once the connection is established, the client will act as a server and answer requests and
    /// notifications in background, using this service.
    pub fn set_service_builder(&mut self, builder: S) -> &mut Self {
        self.service_builder = Some(builder);
        self
    }

    /// Connect to the remote `MessagePack-RPC` endpoint. This consumes the `Connector`.
    pub fn connect(&mut self) -> Connection {
        trace!("Trying to connect to {}.", self.address);

        let (connection, client_tx, error_tx) = Connection::new();

        if self.tls {
            let endpoint = self.tls_connect(client_tx, error_tx);
            self.handle.spawn(endpoint);
        } else {
            let endpoint = self.tcp_connect(client_tx, error_tx);
            self.handle.spawn(endpoint);
        }

        connection
    }

    // FIXME: I don't really understand why the return type is BoxFuture<(), ()>
    // I thought is would be BoxFuture<Endpoint<S, TlsStream<TcpStream>>, ()>
    fn tls_connect(
        &mut self,
        client_tx: oneshot::Sender<Client>,
        error_tx: oneshot::Sender<io::Error>,
    ) -> Box<Future<Item = (), Error = ()>> {
        let tcp_connection = TcpStream::connect(self.address, self.handle);

        let domain = self.tls_domain.take();
        let tls_handshake = tcp_connection.and_then(move |stream| {
            trace!("TCP connection established. Starting TLS handshake.");
            let tls_connector =  TlsConnector::builder().unwrap().build().unwrap();
            if let Some(domain) = domain {
                tls_connector.connect_async(&domain, stream)
            } else {
                tls_connector.danger_connect_async_without_providing_domain_for_certificate_verification_and_server_name_indication(stream)
            }.map_err(|e| { io::Error::new(io::ErrorKind::Other, e) })
        });

        let service_builder = self.service_builder.take();
        let endpoint = tls_handshake
            .and_then(move |stream| {
                trace!("TLS handshake done.");

                let mut endpoint = Endpoint::new(stream);

                let client_proxy = endpoint.set_client();
                if client_tx.send(client_proxy.clone()).is_err() {
                    panic!("Failed to send client to connection.");
                }

                if let Some(service_builder) = service_builder {
                    endpoint.set_server(service_builder.build(client_proxy));
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

        Box::new(endpoint)
    }

    // FIXME: I don't really understand why the return type is BoxFuture<(), ()>
    // I thought is would be BoxFuture<Endpoint<S, TcpStream>, ()>
    fn tcp_connect(
        &mut self,
        client_tx: oneshot::Sender<Client>,
        error_tx: oneshot::Sender<io::Error>,
    ) -> Box<Future<Item = (), Error = ()>> {
        let service_builder = self.service_builder.take();
        let endpoint = TcpStream::connect(self.address, self.handle)
            .and_then(move |stream| {
                trace!("TCP connection established.");

                let mut endpoint = Endpoint::new(stream);

                let client_proxy = endpoint.set_client();
                if client_tx.send(client_proxy.clone()).is_err() {
                    panic!("Failed to send client to connection.");
                }

                if let Some(service_builder) = service_builder {
                    endpoint.set_server(service_builder.build(client_proxy));
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

        Box::new(endpoint)
    }
}

/// A dummy Service that is used for endpoints that act as pure clients, i.e. that do not need to
/// act handle incoming requests or notifications.
struct NoService;

impl Service for NoService {
    type Error = io::Error;
    type T = String;
    type E = String;

    /// Handle a `MessagePack-RPC` request by panicking
    fn handle_request(
        &mut self,
        _method: &str,
        _params: &[Value],
    ) -> Box<Future<Item = Result<Self::T, Self::E>, Error = Self::Error>> {
        panic!("This endpoint does not handle requests");
    }

    /// Handle a `MessagePack-RPC` notification by panicking
    fn handle_notification(
        &mut self,
        _method: &str,
        _params: &[Value],
    ) -> Box<Future<Item = (), Error = Self::Error>> {
        panic!("This endpoint does not handle notifications");
    }
}

impl ServiceBuilder for NoService {
    type Service = NoService;

    fn build(&self, _client: Client) -> Self {
        NoService {}
    }
}

/// A connector for `MessagePack-RPC` clients. Such a connector results in a client that does not
/// handle requests and responses coming from the remote endpoint. It can only _send_ requests and
/// notifications. Incoming requests and notifications will be silently ignored.
///
/// If you need a client that handles incoming requests and notifications, implement a `Service`,
/// and use it with a regular `Connector` (see also:
/// [`Connector::set_service_builder`](struct.Connector.html#method.set_service_builder))
///
/// `ClientOnlyConnector` is just a wrapper around `Connector` to reduce boilerplate for people who
/// only need a basic MessagePack-RPC client.
pub struct ClientOnlyConnector<'a, 'b>(Connector<'a, 'b, NoService>);

impl<'a, 'b> ClientOnlyConnector<'a, 'b> {
    /// Create a new `ClientOnlyConnector`.
    pub fn new(address: &'a SocketAddr, handle: &'b Handle) -> Self {
        ClientOnlyConnector(Connector::<'a, 'b, NoService>::new(address, handle))
    }

    /// Connect to the remote `MessagePack-RPC` server.
    pub fn connect(&mut self) -> Connection {
        self.0.connect()
    }

    /// Enable TLS for this connection. `domain` is the hostname of the remote endpoint so which we
    /// are connecting.
    pub fn set_tls_connector(&mut self, domain: String) -> &mut Self {
        let _ = self.0.set_tls_connector(domain);
        self
    }

    /// Enable TLS for this connection, but without hostname verification. This is dangerous,
    /// because it means that any server with a valid certificate will be trusted. Hence, it is not
    /// recommended.
    pub fn set_tls_connector_with_hostname_verification_disabled(&mut self) -> &mut Self {
        let _ = self.0
            .set_tls_connector_with_hostname_verification_disabled();
        self
    }
}

/// A future that returns a `MessagePack-RPC` endpoint when it completes successfully.
pub struct Connection {
    client_rx: oneshot::Receiver<Client>,
    error_rx: oneshot::Receiver<io::Error>,
}

impl Connection {
    fn new() -> (Self, oneshot::Sender<Client>, oneshot::Sender<io::Error>) {
        let (client_tx, client_rx) = oneshot::channel();
        let (error_tx, error_rx) = oneshot::channel();

        let connection = Connection {
            client_rx: client_rx,
            error_rx: error_rx,
        };

        (connection, client_tx, error_tx)
    }
}

impl Future for Connection {
    type Item = Client;
    type Error = io::Error;

    // FIXME: I'm not sure about the logic is right here.
    //
    // Also, it would be *much* nicer to have only one channel that gives us
    // Result<Client, io::Error> instead of two distinct channels.
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match (self.client_rx.poll(), self.error_rx.poll()) {
            // We have a client, return it
            (Ok(Async::Ready(client)), _) => Ok(Async::Ready(client)),
            // We have an error, return it
            (_, Ok(Async::Ready(e))) => Err(e),
            // Both channels got closed before we received either an error or a client...
            // That should not happen so we panic here:
            (Err(Canceled), Err(Canceled)) => panic!("Failed to poll connection (client dropped?)"),
            // At least one channel is still not ready. The other one may have errored out already.
            // Let's wait for the channel that is still active.
            // I'm not sure this is the right thing to do, because we might wait forever here.
            // Should we return an error instead?
            _ => Ok(Async::NotReady),
        }
    }
}
