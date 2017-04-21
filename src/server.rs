//! Building blocks for building msgpack-rpc servers.
use std::io;
use tokio_service::{NewService, Service};
use std::net::SocketAddr;
use tokio_proto::TcpServer;
use futures::Future;
use message::Message;
use protocol::Protocol;

pub struct MsgpackRpc<T> {
    inner: T,
}

pub fn serve<T>(addr: SocketAddr, new_service: T)
    where T: NewService<Request = Message, Response = Message, Error = io::Error> + Send + Sync + 'static,
{
    let new_service = MsgpackRpc { inner: new_service };
    TcpServer::new(Protocol, addr).serve(new_service);
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
