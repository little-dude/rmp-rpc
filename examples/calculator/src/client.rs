use std::net::SocketAddr;
use rmp_rpc;
use rmp_rpc::msgpack::{Value, Integer};
use futures::Future;
use tokio_core::reactor::Core;
use std::{fmt, io, error};
use futures::sync::oneshot;

pub fn connect(addr: &SocketAddr) -> Result<Client, io::Error> {
    let mut reactor = Core::new()?;
    let handle = reactor.handle();
    let inner_client = reactor.run(rmp_rpc::Client::connect(addr, &handle))?;
    let client = Client {
        client: inner_client,
        reactor: reactor,
    };
    Ok(client)
}

pub type Response = oneshot::Receiver<Result<i64, CalculatorError>>;

pub struct Client {
    client: rmp_rpc::Client,
    reactor: Core,
}

impl Client {
    pub fn add(&self, values: &[i64]) -> Response {
        let params = values.iter().map(|v| Value::Integer(Integer::from(*v))).collect();
        self.call("add", params)
    }

    pub fn sub(&self, values: &[i64]) -> Response {
        let params = values.iter().map(|v| Value::Integer(Integer::from(*v))).collect();
        self.call("sub", params)
    }

    pub fn res(&self) -> Response {
        self.call("res", vec![])
    }

    pub fn clear(&self) -> Response {
        self.call("clear", vec![])
    }

    fn call(&self, method: &str, params: Vec<Value>) -> Response {
        let handle = self.reactor.handle();
        let (tx, rx) = oneshot::channel();
        handle.spawn(self.client
            .request(method, params)
            .then(|response| Ok(tx.send(parse_response(response)).unwrap())));
        rx
    }
}

fn parse_response(response: Result<Result<Value, Value>, io::Error>)
                  -> Result<i64, CalculatorError> {
    match response? {
        Ok(result) => {
            if let Value::Integer(int) = result {
                match int.as_i64() {
                    Some(int) => {
                        return Ok(int);
                    }
                    None => {
                        return Err(CalculatorError::Client(format!("Could not parse server \
                                                                    response as an integer")));
                    }
                }
            } else {
                return Err(CalculatorError::Client(format!("Could not parse server response as \
                                                            an integer")));
            }
        }
        Err(error) => {
            if let Value::String(s) = error {
                match s.as_str() {
                    Some(error_str) => {
                        return Err(CalculatorError::Server(error_str.to_string()));
                    }
                    None => {
                        return Err(CalculatorError::Client(format!("Could not parse server \
                                                                    response as a string")));
                    }
                }
            } else {
                return Err(CalculatorError::Client(format!("Could not parse server response as \
                                                            a string")));
            }
        }
    }
}

#[derive(Debug)]
pub enum CalculatorError {
    /// IO error that occured while communicating with the server.
    Io(io::Error),
    /// Error returned by the server upon a request.
    Server(String),
    /// Error while processing the server response.
    Client(String),
}

impl fmt::Display for CalculatorError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            CalculatorError::Io(ref err) => {
                write!(f,
                       "an IO error occured while communicating with the server: {}",
                       err)
            }
            CalculatorError::Server(ref msg) => write!(f, "the server returned an error: {}", msg),
            CalculatorError::Client(ref msg) => {
                write!(f, "failed to process the server response (reason: {})", msg)
            }
        }
    }
}

impl error::Error for CalculatorError {
    fn description(&self) -> &str {
        match *self {
            CalculatorError::Io(_) => "an IO error occured while communicating with the server",
            CalculatorError::Server(_) => "the server returned an error",
            CalculatorError::Client(_) => "failed to process the server response",
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        match *self {
            CalculatorError::Io(ref e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for CalculatorError {
    fn from(err: io::Error) -> CalculatorError {
        CalculatorError::Io(err)
    }
}
