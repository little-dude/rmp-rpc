use std::collections::HashMap;
use std::io;

use futures::{Async, AsyncSink, Future, IntoFuture, Poll, Sink, StartSend, Stream};
use futures::sync::{mpsc, oneshot};
use tokio_core::reactor;
use tokio_io::codec::Framed;
use tokio_io::{AsyncRead, AsyncWrite};
use rmpv::Value;

use message::{Message, Notification, Request};
use message::Response as MsgPackResponse;
use codec::Codec;

pub trait IntoStaticFuture {
    type Future: Future<Item = Self::Item, Error = Self::Error> + 'static;
    type Item;
    type Error;

    fn into_static_future(self) -> Self::Future;
}

impl<F: IntoFuture> IntoStaticFuture for F
where
    <F as IntoFuture>::Future: 'static,
{
    type Future = <F as IntoFuture>::Future;
    type Item = <F as IntoFuture>::Item;
    type Error = <F as IntoFuture>::Error;

    fn into_static_future(self) -> Self::Future {
        self.into_future()
    }
}

/// The `Service` trait defines how a `MessagePack-RPC` server handles requests and notifications.
pub trait Service {
    /// The type of future returned by `handle_request`. This future will be spawned on the event
    /// loop, and when it is complete then the result will be sent back to the client that made the
    /// request.
    ///
    /// Note that if your `handle_request` method involves only a simple and quick computation,
    /// then you can set `RequestFut` to `Result<Value, Value>` (which gets turned into a future
    /// that completes immediately). You only need to use a "real" future if there's some longer
    /// computation or I/O that needs to be deferred.
    type RequestFuture: IntoStaticFuture<Item = Value, Error = Value>;

    /// The type of future returned by `handle_notification`. This future will be spawned on the
    /// event loop and run to completion. The result of the future will be ignored, since there is
    /// no way to return an error to the client that sent the notification.
    type NotificationFuture: IntoStaticFuture<Item = (), Error = ()>;

    /// Handle a `MessagePack-RPC` request.
    ///
    /// The name of the request is `method`, and the parameters are given in `params`.
    ///
    /// Note that this method is called synchronously within the main event loop, and so it should
    /// return quickly. If you need to run a longer computation, put it in a future and return it.
    fn handle_request(&mut self, method: &str, params: &[Value]) -> Self::RequestFuture;

    /// Handle a `MessagePack-RPC` notification.
    ///
    /// Note that this method is called synchronously within the main event loop, and so it should
    /// return quickly. If you need to run a longer computation, put it in a future and return it.
    fn handle_notification(&mut self, method: &str, params: &[Value]) -> Self::NotificationFuture;
}

/// This is a beefed-up version of [`Service`], in which the various handler methods also get
/// access to a [`Client`], which allows them to send requests and notifications to the same
/// msgpack-rpc client that made the original request.
pub trait ServiceWithClient {
    /// The type of future returned by `handle_request`. See [`Service::handle_request`] for more
    /// information.
    type RequestFuture: IntoStaticFuture<Item = Value, Error = Value>;

    /// The type of future returned by `handle_notification`. See [`Service::handle_notification`]
    /// for more information.
    type NotificationFuture: IntoStaticFuture<Item = (), Error = ()>;

    /// Handle a `MessagePack-RPC` request.
    ///
    /// This differs from [`Service::handle_request`] in that you also get access to a [`Client`]
    /// for sending requests and notifications.
    fn handle_request(
        &mut self,
        client: &mut Client,
        method: &str,
        params: &[Value],
    ) -> Self::RequestFuture;

    /// Handle a `MessagePack-RPC` notification.
    ///
    /// This differs from [`Service::handle_notification`] in that you also get access to a
    /// [`Client`] for sending requests and notifications.
    fn handle_notification(
        &mut self,
        client: &mut Client,
        method: &str,
        params: &[Value],
    ) -> Self::NotificationFuture;
}

// Given a service that doesn't require access to a client, we can also treat it as a service that
// does require access to a client.
impl<S: Service> ServiceWithClient for S {
    type RequestFuture = <S as Service>::RequestFuture;
    type NotificationFuture = <S as Service>::NotificationFuture;

    fn handle_request(
        &mut self,
        _client: &mut Client,
        method: &str,
        params: &[Value],
    ) -> Self::RequestFuture {
        self.handle_request(method, params)
    }

    fn handle_notification(
        &mut self,
        _client: &mut Client,
        method: &str,
        params: &[Value],
    ) -> Self::NotificationFuture {
        self.handle_notification(method, params)
    }
}

struct Server<S: ServiceWithClient> {
    service: S,
    // A handle for spawning new futures.
    // TODO: this will go away once we port to futures 0.2, since then we can spawn from within the
    // poll function
    handle: reactor::Handle,
    // This will receive responses from the service (or possibly from whatever worker tasks that
    // the service spawned). The u32 contains the id of the request that the response is for.
    pending_responses: mpsc::UnboundedReceiver<(u32, Result<Value, Value>)>,
    // We hand out a clone of this whenever we call `service.handle_request`.
    response_sender: mpsc::UnboundedSender<(u32, Result<Value, Value>)>,
    // TODO: We partially add backpressure by ensuring that the pending responses get sent out
    // before we accept new requests. However, it could be that there are lots of response
    // computations out there that haven't sent pending responses yet; we don't yet have a way to
    // apply backpressure there.
}

impl<S: ServiceWithClient> Server<S> {
    fn new(service: S, handle: &reactor::Handle) -> Self {
        let (send, recv) = mpsc::unbounded();

        Server {
            service,
            handle: handle.clone(),
            pending_responses: recv,
            response_sender: send,
        }
    }

    // Pushes all responses (that are ready) onto the stream, to send back to the client.
    //
    // Returns Async::Ready if all of the pending responses were successfully sent on their way.
    // (This does not necessarily mean that they were received yet.)
    fn send_responses<T: AsyncRead + AsyncWrite>(
        &mut self,
        sink: &mut Transport<T>,
    ) -> Poll<(), io::Error> {
        while let Ok(poll) = self.pending_responses.poll() {
            if let Async::Ready(Some((id, result))) = poll {
                let msg = Message::Response(MsgPackResponse { id, result });
                // FIXME: in futures 0.2, use poll_ready before reading from pending_responses, and
                // don't panic here.
                sink.start_send(msg).unwrap();
            } else {
                if let Async::Ready(None) = poll {
                    panic!("we store the sender, it can't be dropped");
                }

                // We're done pushing all messages into the sink, now try to flush it.
                return sink.poll_complete();
            }
        }
        panic!("an UnboundedReceiver should never give an error");
    }

    fn spawn_request_worker<F: Future<Item = Value, Error = Value> + 'static>(
        &self,
        id: u32,
        f: F,
    ) {
        // TODO: avoid spawning if the future is already ready.
        let send = self.response_sender.clone();
        self.handle.spawn(f.then(move |result| {
            send.unbounded_send((id, result))
                    // An error in unbounded_send means that the receiver has been dropped, which
                    // means that the Server has stopped running. There is no meaningful way to
                    // signal an error from here (but the client should see an error anyway,
                    // because its stream will end before it gets a response).
                    .map_err(|_| ())
        }));
    }

    fn spawn_notification_worker<F: Future<Item = (), Error = ()> + 'static>(&self, f: F) {
        // TODO: avoid spawning if the future is already ready.
        self.handle.spawn(f);
    }
}

// We need to write three different endpoints: client, server, and client+server. This trait helps
// us avoid code duplication by defining the two main roles of an endpoint.
trait MessageHandler {
    // We just received `msg` on our input stream. Handle it.
    fn handle_incoming(&mut self, msg: Message);

    // Try to push out all of the outgoing messages (e.g. responses in the case of a server,
    // notifications+requests in the case of a client) onto the sink. Return Ok(Async::Ready(()))
    // if we managed to push them all out and flush the sink.
    fn send_outgoing<T: AsyncRead + AsyncWrite>(
        &mut self,
        sink: &mut Transport<T>,
    ) -> Poll<(), io::Error>;

    // Is the endpoint finished? This is only relevant for clients, since servers and
    // client+servers will never voluntarily stop.
    fn is_finished(&self) -> bool {
        false
    }
}

type ResponseTx = oneshot::Sender<Result<Value, Value>>;
/// Future response to a request. It resolved once the response is available.
// TODO: return an error if the stream has an error
pub struct Response(oneshot::Receiver<Result<Value, Value>>);

type AckTx = oneshot::Sender<()>;

/// A future that resolves when a notification has been effictively sent to the server. It does not
/// guarantees that the server receives it, just that it has been sent.
pub struct Ack(oneshot::Receiver<()>);

// TODO: perhaps make these bounded (for better backpressure)
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
    client_closed: bool,
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
            client_closed: false,
            request_id: 0,
            requests_rx,
            notifications_rx,
            pending_requests: HashMap::new(),
            pending_notifications: Vec::new(),
        };

        (client, client_proxy)
    }

    fn process_notifications<T: AsyncRead + AsyncWrite>(&mut self, stream: &mut Transport<T>) {
        // Don't try to process notifications after the notifications channel was closed, because
        // trying to read from it might cause panics.
        if self.client_closed {
            return;
        }

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
                    self.client_closed = true;
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

    fn send_messages<T: AsyncRead + AsyncWrite>(
        &mut self,
        stream: &mut Transport<T>,
    ) -> Poll<(), io::Error> {
        self.process_requests(stream);
        self.process_notifications(stream);

        match stream.poll_complete()? {
            Async::Ready(()) => {
                self.acknowledge_notifications();
                Ok(Async::Ready(()))
            }
            Async::NotReady => Ok(Async::NotReady),
        }
    }

    fn process_requests<T: AsyncRead + AsyncWrite>(&mut self, stream: &mut Transport<T>) {
        // Don't try to process requests after the requests channel was closed, because
        // trying to read from it might cause panics.
        if self.client_closed {
            return;
        }
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
                    self.client_closed = true;
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

impl<S: Service> MessageHandler for Server<S> {
    fn handle_incoming(&mut self, msg: Message) {
        match msg {
            Message::Request(req) => {
                let f = self.service.handle_request(&req.method, &req.params);
                self.spawn_request_worker(req.id, f.into_static_future());
            }
            Message::Notification(note) => {
                let f = self.service.handle_notification(&note.method, &note.params);
                self.spawn_notification_worker(f.into_static_future());
            }
            Message::Response(_) => {
                trace!("This endpoint doesn't handle responses, ignoring the msg.");
            }
        };
    }

    fn send_outgoing<T: AsyncRead + AsyncWrite>(
        &mut self,
        sink: &mut Transport<T>,
    ) -> Poll<(), io::Error> {
        self.send_responses(sink)
    }
}

impl MessageHandler for InnerClient {
    fn handle_incoming(&mut self, msg: Message) {
        trace!("Received {:?}", msg);
        if let Message::Response(response) = msg {
            self.process_response(response);
        } else {
            trace!("This endpoint only handles reponses, ignoring the msg.");
        }
    }

    fn send_outgoing<T: AsyncRead + AsyncWrite>(
        &mut self,
        sink: &mut Transport<T>,
    ) -> Poll<(), io::Error> {
        self.send_messages(sink)
    }

    fn is_finished(&self) -> bool {
        self.client_closed && self.pending_requests.is_empty()
            && self.pending_notifications.is_empty()
    }
}

struct ClientAndServer<S: ServiceWithClient> {
    inner_client: InnerClient,
    server: Server<S>,
    client: Client,
}

impl<S: ServiceWithClient> MessageHandler for ClientAndServer<S> {
    fn handle_incoming(&mut self, msg: Message) {
        match msg {
            Message::Request(req) => {
                let f =
                    self.server
                        .service
                        .handle_request(&mut self.client, &req.method, &req.params);
                self.server
                    .spawn_request_worker(req.id, f.into_static_future());
            }
            Message::Notification(note) => {
                let f = self.server.service.handle_notification(
                    &mut self.client,
                    &note.method,
                    &note.params,
                );
                self.server
                    .spawn_notification_worker(f.into_static_future());
            }
            Message::Response(response) => self.inner_client.process_response(response),
        };
    }

    fn send_outgoing<T: AsyncRead + AsyncWrite>(
        &mut self,
        sink: &mut Transport<T>,
    ) -> Poll<(), io::Error> {
        if let Async::Ready(_) = self.server.send_responses(sink)? {
            self.inner_client.send_messages(sink)
        } else {
            Ok(Async::NotReady)
        }
    }
}

struct InnerEndpoint<MH: MessageHandler, T: AsyncRead + AsyncWrite> {
    handler: MH,
    stream: Transport<T>,
}

impl<MH: MessageHandler, T: AsyncRead + AsyncWrite> Future for InnerEndpoint<MH, T> {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // Try to flush out all the responses that are queued up. If this doesn't succeed yet, our
        // output sink is full. In that case, we'll apply some backpressure to our input stream by
        // not reading from it.
        if let Async::NotReady = self.handler.send_outgoing(&mut self.stream)? {
            trace!("Sink not yet flushed, waiting...");
            return Ok(Async::NotReady);
        }

        trace!("Polling stream.");
        while let Async::Ready(msg) = self.stream.poll()? {
            if let Some(msg) = msg {
                self.handler.handle_incoming(msg);
            } else {
                trace!("Stream closed by remote peer.");
                // FIXME: not sure if we should still continue sending responses here. Is it
                // possible that the client closed the stream only one way and is still waiting
                // for response? Not for TCP at least, but maybe for other transport types?
                return Ok(Async::Ready(()));
            }
        }

        if self.handler.is_finished() {
            trace!("inner client finished, exiting...");
            Ok(Async::Ready(()))
        } else {
            trace!("notifying the reactor that we're not done yet");
            Ok(Async::NotReady)
        }
    }
}

/// A `Future` in charge of sending and receiving messages on behalf of a client.
///
/// The main way to create a magpack-rpc client is to call [`ClientEndpoint::new`] to make a
/// `(ClientEndpoint, Client)` pair. The `Client` half of the pair can be used to send requests and
/// notifications. The `ClientEndpoint` half of the pair needs to be spawned onto a task in order
/// to do the job of actually pushing the messages out and processing the responses.
///
/// TODO: example
pub struct ClientEndpoint<T: AsyncRead + AsyncWrite> {
    inner: InnerEndpoint<InnerClient, T>,
}

impl<T: AsyncRead + AsyncWrite> ClientEndpoint<T> {
    /// Creates a new `ClientEndpoint`, along with a `Client` that can be used to send requests and
    /// notifications.
    pub fn new(stream: T) -> (Self, Client) {
        let (inner_client, client) = InnerClient::new();
        let endpoint = ClientEndpoint {
            inner: InnerEndpoint {
                stream: Transport(stream.framed(Codec)),
                handler: inner_client,
            },
        };
        (endpoint, client)
    }
}

impl<T: AsyncRead + AsyncWrite> Future for ClientEndpoint<T> {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll()
    }
}

/// Creates a future for running a `Service` on a stream.
///
/// The returned future will run until the stream is closed; if the stream encounters an error,
/// then the future will propagate it and terminate.
pub fn serve<'a, S: Service + 'a, T: AsyncRead + AsyncWrite + 'a>(
    stream: T,
    service: S,
    handle: &reactor::Handle,
) -> Box<Future<Item = (), Error = io::Error> + 'a> {
    Box::new(ServerEndpoint::new(stream, service, handle))
}

struct ServerEndpoint<S: Service, T: AsyncRead + AsyncWrite> {
    inner: InnerEndpoint<Server<S>, T>,
}

impl<S: Service, T: AsyncRead + AsyncWrite> ServerEndpoint<S, T> {
    pub fn new(stream: T, service: S, handle: &reactor::Handle) -> Self {
        ServerEndpoint {
            inner: InnerEndpoint {
                stream: Transport(stream.framed(Codec)),
                handler: Server::new(service, handle),
            },
        }
    }
}

impl<S: Service, T: AsyncRead + AsyncWrite> Future for ServerEndpoint<S, T> {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll()
    }
}

/// A `Future` for running both a client and a server at the same time.
///
/// The client part will be provided to the [`ServiceWithClient::handle_request`] and
/// [`ServiceWithClient::handle_notification`] functions, so that the server can send back requests
/// and notifications as part of its handling duties. You may also access the client with the
/// [`client()`] function if you want to send additional requests.
///
/// The returned future needs to be spawned onto a task in order to actually run the server (and
/// the client). It will run until the stream is closed; if the stream encounters an error, the
/// future will propagate it and terminate.
///
/// TODO: example
pub struct Endpoint<S: ServiceWithClient, T: AsyncRead + AsyncWrite> {
    inner: InnerEndpoint<ClientAndServer<S>, T>,
}

impl<S: ServiceWithClient, T: AsyncRead + AsyncWrite> Endpoint<S, T> {
    /// Creates a new `Endpoint` on `stream`, using `service` to handle requests and notifications.
    pub fn new(stream: T, service: S, handle: &reactor::Handle) -> Self {
        let (inner_client, client) = InnerClient::new();
        Endpoint {
            inner: InnerEndpoint {
                stream: Transport(stream.framed(Codec)),
                handler: ClientAndServer {
                    inner_client,
                    client,
                    server: Server::new(service, handle),
                },
            },
        }
    }

    /// Returns a handle to the client half of this `Endpoint`, which can be used for sending
    /// requests and notifications.
    pub fn client(&self) -> Client {
        self.inner.handler.client.clone()
    }
}

impl<S: ServiceWithClient, T: AsyncRead + AsyncWrite> Future for Endpoint<S, T> {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll()
    }
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
            requests_tx,
            notifications_tx,
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
