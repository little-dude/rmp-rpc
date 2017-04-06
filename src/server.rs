use std::io;
use tokio_proto::pipeline::ServerProto;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::Framed;
use codec::Codec;
use message::Message;

pub struct MsgPackRpcServerProto;

impl<T: AsyncRead + AsyncWrite + 'static> ServerProto<T> for MsgPackRpcServerProto {
    type Request = Message;
    type Response = Message;
    type Transport = Framed<T, Codec>;
    type BindTransport = Result<Self::Transport, io::Error>;
    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(Codec))
    }
}

