use std::{error, fmt, io};
use rmpv::{encode, decode};

pub struct EncodeError(io::Error);

impl From<encode::Error> for EncodeError {
    fn from(err: encode::Error) -> EncodeError {
        match err {
            encode::Error::InvalidMarkerWrite(io_err) |
            encode::Error::InvalidDataWrite(io_err) => EncodeError(io_err),
        }
    }
}

impl From<EncodeError> for io::Error {
    fn from(err: EncodeError) -> io::Error {
        err.0
    }
}


/// Error while decoding a sequence of bytes into a `MessagePack-RPC` message
#[derive(Debug)]
pub enum DecodeError {
    /// Some bytes are missing to decode a full msgpack value
    Truncated,

    /// A byte sequence could no be decoded as a msgpack value
    Malformed,

    /// An unknown IO error while reading a byte sequence
    UnknownIo(io::Error),

    /// A byte sequence could be decoded as a msgpack value, but this value is not a valid
    /// msgpack-rpc message.
    Invalid,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        error::Error::description(self).fmt(f)
    }
}


impl error::Error for DecodeError {
    fn description(&self) -> &str {
        match *self {
            DecodeError::Truncated => "could not read enough bytes to decode a complete message",
            DecodeError::UnknownIo(_) => "Unknown IO error while decoding a message",
            DecodeError::Invalid => {
                "the message could be decoded but is not a valid msgpack-rpc message"
            }
            DecodeError::Malformed => "A byte sequence could no be decoded as a msgpack value",
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        match *self {
            DecodeError::UnknownIo(ref e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for DecodeError {
    fn from(err: io::Error) -> DecodeError {
        match err.kind() {
            io::ErrorKind::UnexpectedEof => DecodeError::Truncated,
            io::ErrorKind::Other => {
                if let Some(cause) = err.get_ref().unwrap().cause() {
                    if cause.description() == "type mismatch" {
                        return DecodeError::Malformed;
                    }
                }
                DecodeError::UnknownIo(err)
            }
            _ => DecodeError::UnknownIo(err),

        }
    }
}

impl From<decode::Error> for DecodeError {
    fn from(err: decode::Error) -> DecodeError {
        match err {
            decode::Error::InvalidMarkerRead(io_err) |
            decode::Error::InvalidDataRead(io_err) => From::from(io_err),
        }
    }
}
