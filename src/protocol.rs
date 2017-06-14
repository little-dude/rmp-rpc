use tokio_proto::pipeline::ClientProto;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::Framed;
use message::Message;
use codec::Codec;
use std::io;

pub struct Protocol;

impl<T: AsyncRead + AsyncWrite + 'static> ClientProto<T> for Protocol {
    type Request = Message;
    type Response = Message;
    type Transport = Framed<T, Codec>;
    type BindTransport = Result<Self::Transport, io::Error>;
    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(Codec))
    }
}
