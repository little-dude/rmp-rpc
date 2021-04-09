use crate::errors::*;
use rmpv::{decode, encode, Integer, Utf8String, Value};
use std::convert::From;
use std::io::{self, Read};

/// Represents a `MessagePack-RPC` message as described in the
/// [specifications](https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md#messagepack-rpc-protocol-specification).
#[derive(PartialEq, Clone, Debug)]
pub(crate) enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

/// Represents a `MessagePack-RPC` request as described in the
/// [specifications](https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md#messagepack-rpc-protocol-specification).
///
/// A request is a message that a client sends to a server when it expects a response. Sending a
/// request is like calling a method: it includes a method name and an array of parameters. The
/// response is like the return value.
#[derive(PartialEq, Clone, Debug)]
pub(crate) struct Request {
    /// The `id` is used to associate a response with a request. If a client sends a request with a
    /// particular `id`, the server should send a response with the same `id`.
    pub id: u32,
    /// A string representing the method name.
    pub method: String,
    /// An array of parameters to the method.
    pub params: Vec<Value>,
}

/// Represents a `MessagePack-RPC` response as described in the
/// [specifications](https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md#messagepack-rpc-protocol-specification).
///
/// After a client sends a [`Request`], the server will send a response back.
#[derive(PartialEq, Clone, Debug)]
pub(crate) struct Response {
    /// The `id` of the [`Request`] that triggered this response.
    pub id: u32,
    /// The result of the [`Request`] that triggered this response.
    pub result: Result<Value, Value>,
}

/// Represents a `MessagePack-RPC` notification as described in the
/// [specifications](https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md#messagepack-rpc-protocol-specification).
///
/// A notification is a message that a client sends to a server when it doesn't expect a response.
/// Sending a notification is like calling a method with no return value: the notification includes
/// a method name and an array of parameters.
#[derive(PartialEq, Clone, Debug)]
pub(crate) struct Notification {
    /// A string representing the method name.
    pub method: String,
    /// An array of parameters to the method.
    pub params: Vec<Value>,
}

const REQUEST_MESSAGE: u64 = 0;
const RESPONSE_MESSAGE: u64 = 1;
const NOTIFICATION_MESSAGE: u64 = 2;

impl Message {
    pub(crate) fn decode<R>(rd: &mut R) -> Result<Message, DecodeError>
    where
        R: Read,
    {
        let msg = decode::value::read_value(rd)?;
        if let Value::Array(ref array) = msg {
            if array.len() < 3 {
                // notification are the shortest message and have 3 items
                return Err(DecodeError::Invalid);
            }
            if let Value::Integer(msg_type) = array[0] {
                match msg_type.as_u64() {
                    Some(REQUEST_MESSAGE) => Ok(Message::Request(Request::decode(array)?)),
                    Some(RESPONSE_MESSAGE) => Ok(Message::Response(Response::decode(array)?)),
                    Some(NOTIFICATION_MESSAGE) => {
                        Ok(Message::Notification(Notification::decode(array)?))
                    }
                    _ => Err(DecodeError::Invalid),
                }
            } else {
                Err(DecodeError::Invalid)
            }
        } else {
            Err(DecodeError::Invalid)
        }
    }

    pub fn as_value(&self) -> Value {
        match *self {
            Message::Request(Request {
                id,
                ref method,
                ref params,
            }) => Value::Array(vec![
                Value::Integer(Integer::from(REQUEST_MESSAGE)),
                Value::Integer(Integer::from(id)),
                Value::String(Utf8String::from(method.as_str())),
                Value::Array(params.clone()),
            ]),
            Message::Response(Response { id, ref result }) => {
                let (error, result) = match *result {
                    Ok(ref result) => (Value::Nil, result.to_owned()),
                    Err(ref err) => (err.to_owned(), Value::Nil),
                };
                Value::Array(vec![
                    Value::Integer(Integer::from(RESPONSE_MESSAGE)),
                    Value::Integer(Integer::from(id)),
                    error,
                    result,
                ])
            }
            Message::Notification(Notification {
                ref method,
                ref params,
            }) => Value::Array(vec![
                Value::Integer(Integer::from(NOTIFICATION_MESSAGE)),
                Value::String(Utf8String::from(method.as_str())),
                Value::Array(params.to_owned()),
            ]),
        }
    }

    pub fn pack(&self) -> io::Result<Vec<u8>> {
        let mut bytes = vec![];
        encode::write_value(&mut bytes, &self.as_value())?;
        Ok(bytes)
    }
}

impl Notification {
    fn decode(array: &[Value]) -> Result<Self, DecodeError> {
        if array.len() < 3 {
            return Err(DecodeError::Invalid);
        }

        let method = if let Value::String(ref method) = array[1] {
            method
                .as_str()
                .map(|s| s.to_string())
                .ok_or(DecodeError::Invalid)?
        } else {
            return Err(DecodeError::Invalid);
        };

        let params = if let Value::Array(ref params) = array[2] {
            params.clone()
        } else {
            return Err(DecodeError::Invalid);
        };

        Ok(Notification { method, params })
    }
}

impl Request {
    fn decode(array: &[Value]) -> Result<Self, DecodeError> {
        if array.len() < 4 {
            return Err(DecodeError::Invalid);
        }

        let id = if let Value::Integer(id) = array[1] {
            id.as_u64()
                .map(|id| id as u32)
                .ok_or(DecodeError::Invalid)?
        } else {
            return Err(DecodeError::Invalid);
        };

        let method = if let Value::String(ref method) = array[2] {
            method
                .as_str()
                .map(|s| s.to_string())
                .ok_or(DecodeError::Invalid)?
        } else {
            return Err(DecodeError::Invalid);
        };

        let params = if let Value::Array(ref params) = array[3] {
            params.clone()
        } else {
            return Err(DecodeError::Invalid);
        };

        Ok(Request { id, method, params })
    }
}

impl Response {
    fn decode(array: &[Value]) -> Result<Self, DecodeError> {
        if array.len() < 2 {
            return Err(DecodeError::Invalid);
        }

        let id = if let Value::Integer(id) = array[1] {
            id.as_u64()
                .map(|id| id as u32)
                .ok_or(DecodeError::Invalid)?
        } else {
            return Err(DecodeError::Invalid);
        };

        match array[2] {
            Value::Nil => Ok(Response {
                id,
                result: Ok(array[3].clone()),
            }),
            ref error => Ok(Response {
                id,
                result: Err(error.clone()),
            }),
        }
    }
}

#[test]
fn test_decode_request() {
    let valid = Message::Request(Request {
        id: 1234,
        method: "dummy".to_string(),
        params: Vec::new(),
    });
    let bytes = valid.pack().unwrap();

    // valid message
    {
        let mut buf = io::Cursor::new(&bytes);
        assert_eq!(valid, Message::decode(&mut buf).unwrap());
    }

    // truncated
    {
        let bytes = Vec::from(&bytes[0..bytes.len() - 1]);
        let mut buf = io::Cursor::new(&bytes);
        assert!(matches!(
            Message::decode(&mut buf),
            Err(DecodeError::Truncated)
        ));
    }

    // invalid message type
    {
        let mut bytes = Vec::from(&bytes[..]);
        bytes[1] = 5;
        let mut buf = io::Cursor::new(&bytes);
        assert!(matches!(
            Message::decode(&mut buf),
            Err(DecodeError::Invalid)
        ));
    }
}
