use tokio_proto::pipeline::ServerProto;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::Framed;
use message::Message;
use codec::Codec;
use std::io;

/// The `MessagePack-RPC` transport protocol.
/// See [the `tokio_proto`
/// documentation](https://tokio-rs.github.io/tokio-proto/tokio_proto/index.html)
///
/// # Examples
///
/// ```
/// use tokio_proto::TcpServer;
/// fn main() {
///     let addr = "127.0.0.1:54321".parse().unwrap();
///     let tcp_server = TcpServer::new(Protocol, addr);
///     // ...
/// }
/// ```
///
pub struct Protocol;

impl<T: AsyncRead + AsyncWrite + 'static> ServerProto<T> for Protocol {
    type Request = Message;
    type Response = Message;
    type Transport = Framed<T, Codec>;
    type BindTransport = Result<Self::Transport, io::Error>;
    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(Codec))
    }
}
