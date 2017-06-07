//! Building blocks for building msgpack-rpc servers.
use std::io;
use tokio_service::{NewService, Service};
use std::net::SocketAddr;
use tokio_proto::TcpServer;
use futures::Future;
use message::Message;
 // use protocol::Protocol;
use protocol::{ServerProto};

struct MsgpackRpc<T> {
    inner: T,
}

/// Start a msgpack-RPC server and block until the server finishes.
///
/// A server must implement
/// [`tokio_service::NewService`](https://tokio-rs.github.io/tokio-service/tokio_service/trait.NewService.html)
/// and [`tokio_service::Service`](https://tokio-rs.github.io/tokio-service/tokio_service/trait.Service.html).
///
/// # Examples
///
/// ```rust,no_run
/// extern crate futures;
/// extern crate tokio_service;
/// extern crate rmp_rpc;
/// 
/// use std::io;
/// use std::net::SocketAddr;
/// use tokio_service::{NewService, Service};
/// use rmp_rpc::{serve, Message};
/// use futures::Future;
/// 
/// 
/// #[derive(Clone)]
/// pub struct MyServer;
/// 
/// impl NewService for MyServer {
///     type Request = Message;
///     type Response = Message;
///     type Error = io::Error;
///     type Instance = MyServer;
/// 
///     fn new_service(&self) -> io::Result<Self::Instance> {
///         Ok(self.clone())
///     }
/// }
/// 
/// impl Service for MyServer {
///     type Request = Message;
///     type Response = Message;
///     type Error = io::Error;
///     type Future = Box<Future<Item=Message, Error=io::Error>>;
/// 
///     fn call(&self, msg: Message) -> Self::Future {
///         match msg {
///             Message::Request { .. } => {
///                 // Handle the request
///                 unimplemented!()
///             }
///             Message::Notification { .. } => {
///                 // Handle the notification
///                 unimplemented!()
///             }
///             Message::Response { .. } => {
///                 // A server is not expecting a response. Handle the error
///                 unimplemented!()
///             }
///         }
///     }
/// }
/// 
/// fn main() { serve("127.0.0.1:12345".parse().unwrap(), MyServer) }
/// ```
///
pub fn serve<T>(addr: SocketAddr, new_service: T)
    where T: NewService<Request = Message, Response = Message, Error = io::Error> + Send + Sync + 'static,
{
    let new_service = MsgpackRpc { inner: new_service };
    TcpServer::new(ServerProto {}, addr).serve(new_service);
}

impl<T> Service for MsgpackRpc<T>
    where T: Service<Request = Message, Response = Message, Error = io::Error>,
          T::Future: 'static
{
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Future = Box<Future<Item = Message, Error = io::Error>>;

    fn call(&self, req: Message) -> Self::Future {
        println!("call");
        Box::new(self.inner.call(req))
    }
}

impl<T> NewService for MsgpackRpc<T>
    where T: NewService<Request = Message, Response = Message, Error = io::Error>,
          <T::Instance as Service>::Future: 'static
{
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Instance = MsgpackRpc<T::Instance>;

    fn new_service(&self) -> io::Result<Self::Instance> {
        let inner = try!(self.inner.new_service());
        Ok(MsgpackRpc { inner: inner })
    }
}
