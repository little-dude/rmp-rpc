use std::net::SocketAddr;
use rmp_rpc::{self, ClientOnlyConnector, Integer, Value};
use futures::Future;
use tokio_core::reactor::Handle;
use std::{error, fmt};

pub type Response = Box<Future<Item = i64, Error = RpcError>>;

pub struct Client(rmp_rpc::Client);

impl Client {
    pub fn connect(
        addr: &SocketAddr,
        handle: &Handle,
    ) -> Box<Future<Item = Self, Error = RpcError>> {
        let client = ClientOnlyConnector::new(addr, handle)
            .connect()
            .map(Client)
            .map_err(|_| ())
            .map_err(From::from);
        Box::new(client)
    }

    pub fn add(&self, values: &[i64]) -> Response {
        let params = values
            .iter()
            .map(|v| Value::Integer(Integer::from(*v)))
            .collect();
        self.request("add", params)
    }

    pub fn sub(&self, values: &[i64]) -> Response {
        let params = values
            .iter()
            .map(|v| Value::Integer(Integer::from(*v)))
            .collect();
        self.request("sub", params)
    }

    pub fn res(&self) -> Response {
        self.request("res", vec![])
    }

    pub fn clear(&self) -> Response {
        self.request("clear", vec![])
    }

    fn request(&self, method: &str, params: Vec<Value>) -> Response {
        Box::new(self.0.request(method, &params).then(parse_response))
    }
}

fn parse_response(response: Result<Result<Value, Value>, ()>) -> Result<i64, RpcError> {
    match response? {
        Ok(result) => if let Value::Integer(int) = result {
            int.as_i64().ok_or_else(|| {
                RpcError::Client(
                    "Could not parse server response as \
                     an integer"
                        .to_string(),
                )
            })
        } else {
            Err(RpcError::Client(
                "Could not parse server response as an integer".to_string(),
            ))
        },
        Err(error) => if let Value::String(s) = error {
            match s.as_str() {
                Some(error_str) => Err(RpcError::Server(error_str.to_string())),
                None => Err(RpcError::Client(
                    "Could not parse server response as a \
                     string"
                        .to_string(),
                )),
            }
        } else {
            Err(RpcError::Client(
                "Could not parse server response as a string".to_string(),
            ))
        },
    }
}

#[derive(Debug)]
pub enum RpcError {
    Other,
    /// Error returned by the server upon a request.
    Server(String),
    /// Error while processing the server response.
    Client(String),
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            RpcError::Other => write!(f, "unknown error"),
            RpcError::Server(ref msg) => write!(f, "the server returned an error: {}", msg),
            RpcError::Client(ref msg) => {
                write!(f, "failed to process the server response (reason: {})", msg)
            }
        }
    }
}

impl error::Error for RpcError {
    fn description(&self) -> &str {
        match *self {
            RpcError::Other => "unknown error",
            RpcError::Server(_) => "the server returned an error",
            RpcError::Client(_) => "failed to process the server response",
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

impl From<()> for RpcError {
    fn from(_: ()) -> RpcError {
        RpcError::Other
    }
}
