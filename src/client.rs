use futures::Future;

use tokio_core::net::TcpStream;
use tokio_core::reactor::Handle;
use tokio_proto::TcpClient;
use tokio_proto::multiplex::ClientService;
use tokio_service::Service;

use std::io;
use std::net::SocketAddr;
use message::{Message, Request};
use protocol::Protocol;
use rmpv::Value;

struct Inner(ClientService<TcpStream, Protocol>);

pub struct Client {
    inner: Inner,
}

pub type Response = Box<Future<Item = Result<Value, Value>, Error = io::Error>>;

impl Client {
    pub fn connect(addr: &SocketAddr,
                   handle: &Handle)
                   -> Box<Future<Item = Client, Error = io::Error>> {
        let ret = TcpClient::new(Protocol)
            .connect(addr, handle)
            .map(|client_service| Client { inner: Inner(client_service) });

        Box::new(ret)
    }

    pub fn call(&self, method: &str, params: Vec<Value>) -> Response {
        let req = Message::Request(Request {
            // we can set this to 0 because under the hood it's handle by tokio at the
            // protocol/codec level
            id: 0,
            method: method.to_string(),
            params: params,
        });
        let resp = self.inner
            .call(req)
            .and_then(|resp| {
                match resp {
                    Message::Response(response) => Ok(response.result),
                    _ => {
                        // not sure what to do here.
                        // i don't think this can happen, so let's just panic for now
                        panic!("Response is not a Message::Response");
                    }
                }
            });
        Box::new(resp) as Response
    }
}

impl Service for Inner {
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Future = Box<Future<Item = Message, Error = io::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        Box::new(self.0.call(req))
    }
}
