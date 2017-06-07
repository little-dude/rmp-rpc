use futures::Future;

use tokio_core::net::TcpStream;
use tokio_core::reactor::Handle;
use tokio_proto::TcpClient;
use tokio_proto::pipeline::ClientService;
use tokio_service::Service;

use std::io;
use std::net::SocketAddr;
use message::Message;
use protocol::Protocol;
use rmpv::Value;


/// Represent a TCP msgpack-rpc client.
pub struct Client(ClientService<TcpStream, Protocol>);

/// A future corresponding to a server response.
pub type Response = Box<Future<Item = Result<Value, Value>, Error = io::Error>>;

impl Client {
    /// Connect to a remote server.
    /// The client returned can be used to perform requests.
    pub fn connect(addr: &SocketAddr,
                   handle: &Handle)
                   -> Box<Future<Item = Client, Error = io::Error>> {
        let ret = TcpClient::new(Protocol)
            .connect(addr, handle)
            .map(Client);
        Box::new(ret)
    }

    /// Perform a msgpack-rpc request.
    pub fn request(&self, method: &str, params: Vec<Value>) -> Response {
        let req = Message::Request {
            // we can set this to 0 because under the hood it's handle by tokio at the
            // protocol/codec level
            id: 0,
            method: method.to_string(),
            params: params,
        };
        let resp = self.call(req).and_then(|resp| {
            match resp {
                Message::Response { result, .. } => Ok(result),
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

impl Service for Client {
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Future = Box<Future<Item = Message, Error = io::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        Box::new(self.0.call(req))
    }
}
