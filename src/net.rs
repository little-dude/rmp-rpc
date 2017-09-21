use futures::{Async, Canceled, Future, Poll, Stream};
use futures::sync::oneshot;
use tokio_core::reactor::Handle;
use tokio_tls::TlsConnectorExt;
use tokio_core::net::{TcpListener, TcpStream};
use std::net::SocketAddr;
use rmpv::Value;
use std::io;

use native_tls::TlsConnector;
use endpoint::{Client, Endpoint, Service, ServiceBuilder};

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
