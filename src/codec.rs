use std::io;
use rmpv;
use bytes::{BufMut, BytesMut};
use tokio_io::codec::{Decoder, Encoder};
use errors::DecodeError;
use message::Message;

pub struct Codec;

impl Decoder for Codec {
    type Item = Message;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        let res: Result<Option<Self::Item>, Self::Error>;
        let position = {
            let mut buf = io::Cursor::new(&src);
            loop {
                match Message::decode(&mut buf) {
                    Ok(message) => {
                        res = Ok(Some(message));
                        break;
                    }
                    Err(err) => match err {
                        DecodeError::Truncated => return Ok(None),
                        DecodeError::Invalid => continue,
                        DecodeError::UnknownIo(io_err) => {
                            res = Err(io_err);
                            break;
                        }
                    },
                }
            }
            buf.position() as usize
        };
        let _ = src.split_to(position);
        res
    }
}

impl Encoder for Codec {
    type Item = Message;
    type Error = io::Error;

    fn encode(&mut self, msg: Self::Item, buf: &mut BytesMut) -> io::Result<()> {
        Ok(rmpv::encode::write_value(
            &mut buf.writer(),
            &msg.as_value(),
        )?)
    }
}

#[test]
fn decode() {
    use message::{Message, Request};
    fn try_decode(input: &[u8], rest: &[u8]) -> io::Result<Option<Message>> {
        let mut codec = Codec {};
        let mut buf = BytesMut::from(input);
        let result = codec.decode(&mut buf);
        assert_eq!(rest, &buf);
        result
    }

    let msg = Message::Request(Request {
        id: 1234,
        method: "dummy".to_string(),
        params: Vec::new(),
    });

    // A single message, nothing is left
    assert_eq!(try_decode(&msg.pack(), b"").unwrap(), Some(msg.clone()));

    // The first message is decoded, the second stays in the buffer
    let mut bytes = [&msg.pack()[..], &msg.pack()[..]].concat();
    assert_eq!(try_decode(&bytes, &msg.pack()).unwrap(), Some(msg.clone()));

    // An incomplete message: nothing gets out and everything stays
    let packed_msg = msg.pack();
    bytes = Vec::from(&packed_msg[0..packed_msg.len() - 1]);
    assert_eq!(try_decode(&bytes, &bytes).unwrap(), None);

    // An invalid message: it gets eaten, and the next message get read.
    bytes = [&vec![0, 1, 2], &msg.pack()[..]].concat();
    assert_eq!(try_decode(&bytes, b"").unwrap(), Some(msg.clone()));
}
