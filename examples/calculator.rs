extern crate tokio_core;
extern crate futures;
extern crate tokio_proto;
extern crate rmp_rpc;


mod server {
    use rmp_rpc::{Dispatch};
    use rmp_rpc::msgpack::{Value, Utf8String, Integer};

    fn argument_error() -> Value {
        Value::String(Utf8String::from("Invalid arguments"))
    }


    #[derive(Clone)]
    pub struct Calculator {
        value: i64,
    }

    impl Calculator {
        pub fn new() -> Self {
            Calculator { value: 0 }
        }

        fn parse_args(params: &[Value]) -> Result<Vec<i64>, Value> {
            let mut ret = vec![];
            for value in params {
                let int = if let Value::Integer(int) = *value {
                    int.as_i64().ok_or(argument_error())?
                } else {
                    return Err(argument_error());
                };
                ret.push(int);
            }
            Ok(ret)
        }

        fn clear(&mut self) -> Result<Value, Value> {
            println!("server: clear");
            self.value = 0;
            self.res()
        }

        fn res(&self) -> Result<Value, Value> {
            println!("server: res");
            Ok(Value::Integer(Integer::from(self.value)))
        }

        fn add(&mut self, params: &[Value]) -> Result<Value, Value> {
            println!("server: add");
            self.value += Self::parse_args(params)?
                .iter()
                .fold(0, |acc, &int| acc + int);
            self.res()
        }

        fn sub(&mut self, params: &[Value]) -> Result<Value, Value> {
            println!("server: sub");
            self.value -= Self::parse_args(params)?
                .iter()
                .fold(0, |acc, &int| acc - int);
            self.res()
        }
    }

    impl Dispatch for Calculator {
        fn dispatch(&mut self, method: &str, params: &[Value]) -> Result<Value, Value> {
            match method {
                "add" | "+" => { self.add(params) },
                "sub" | "-" => { self.sub(params) },
                "res" | "=" => { self.res() },
                "clear" => { self.clear() },
                _ => {
                    Err(Value::String(Utf8String::from(format!("Invalid method {}", method))))
                }
            }
        }
    }
}

mod client {
    use std::net::SocketAddr;
    use rmp_rpc;
    use rmp_rpc::msgpack::{Value, Integer};
    use futures::Future;
    use tokio_core::reactor::Core;
    use std::io;
    use futures::sync::oneshot;

    fn parse_response(value: Result<Value, Value>) ->  Result<Result<i64, String>, io::Error> {
        match value {
            Ok(res) => {
                if let Value::Integer(int) = res {
                    match int.as_i64() {
                        Some(int) => {
                            return Ok(Ok(int));
                        }
                        None => {
                            return Ok(Err(format!("Could not parse server response as an integer")));
                        }
                    }
                } else {
                    Ok(Err(format!("Could not parse server response as an integer")))
                }
            }
            Err(err) => {
                if let Value::String(s) = err {
                    match s.as_str() {
                        Some(err_str) => {
                            return Ok(Err(err_str.to_string()));
                        }
                        None => {
                            return Ok(Err(format!("Could not parse server response as a string")));
                        }
                    }
                } else {
                    Ok(Err(format!("Could not parse server response as a string")))
                }
            }
        }
    }

    fn send_result(result: Result<Result<i64, String>, io::Error>,
                   chan: oneshot::Sender<Result<Result<i64, String>, io::Error>>) {
        chan.send(result).unwrap()
    }

    // For a real client, this should probably be simplified into
    // oneshot::Receiver<Result<i64, CustomError>>
    pub type Response = oneshot::Receiver<Result<Result<i64, String>, io::Error>>;

    pub struct Client {
        client: Option<rmp_rpc::Client>,
        address: SocketAddr,
        reactor: Core,
    }

    impl Client {

        pub fn new(addr: &SocketAddr) -> Self {
            Client {
                client: None,
                address: addr.clone(),
                reactor: Core::new().unwrap(),
            }
        }

        pub fn connect(&mut self) -> Result<(), io::Error> {
            println!("client: connecting");
            let handle = self.reactor.handle();
            let inner_client = self.reactor.run(rmp_rpc::Client::connect(&self.address, &handle))?;
            self.client = Some(inner_client);
            println!("client: connected");
            Ok(())
        }

        pub fn add(&self, values: &[i64]) -> Response {
            println!("client: add");
            let handle = self.reactor.handle();
            let (tx, rx) = oneshot::channel();

            println!("spawning future");
            handle.spawn(
                // call rmp_rpc::Client::request to send the request.
                self.client.as_ref().unwrap()

                // send the request
                .request("add", values.iter().map(|v| Value::Integer(Integer::from(*v))).collect())

                // parse the response, since we don't want users to deal with
                // rmp_rpc::msgpack::Value directly.
                .and_then(|response| parse_response(response))

                // Handle::spawn requires the future to handle errors, and result internally
                // so we just send the response through the channel, and return the receiving end
                // of the channel.
                .then(|result| Ok(send_result(result, tx))));
            println!("future spawned");

            // return the channel. Users can just call `wait` on this to block until the future
            // finishes and the result is sent.
            rx
        }

        pub fn sub(&self, values: &[i64]) -> Response {
            println!("client: sub");
            let handle = self.reactor.handle();
            let (tx, rx) = oneshot::channel();
            handle.spawn(
                self.client.as_ref().unwrap()
                .request("sub", values.iter().map(|v| Value::Integer(Integer::from(*v))).collect())
                .and_then(|response| parse_response(response))
                .then(|result| Ok(send_result(result, tx))));
            rx
        }

        pub fn res(&self) -> Response {
            println!("client: res");
            let handle = self.reactor.handle();
            let (tx, rx) = oneshot::channel();
            handle.spawn(
                self.client.as_ref().unwrap()
                .request("res", vec![])
                .and_then(|response| parse_response(response))
                .then(|result| Ok(send_result(result, tx))));
            rx
        }

        pub fn clear(&self) -> Response {
            println!("client: clear");
            let handle = self.reactor.handle();
            let (tx, rx) = oneshot::channel();
            handle.spawn(
                self.client.as_ref().unwrap()
                .request("clear", vec![])
                .and_then(|response| parse_response(response))
                .then(|result| Ok(send_result(result, tx))));
            rx
        }
    }
}


fn main() {
    use client::Client;
    use server::Calculator;
    use tokio_proto::TcpServer;
    use rmp_rpc::{Protocol, Server};
    use futures::Future;
    use std::thread;
    use std::time::Duration;

    let addr = "127.0.0.1:54321".parse().unwrap();

    thread::spawn(move || {
        let tcp_server = TcpServer::new(Protocol, addr);
        tcp_server.serve(|| {
            Ok(Server::new(Calculator::new()))
        });
    });

    thread::sleep(Duration::from_millis(100));

    let mut client = Client::new(&addr);
    client.connect().unwrap();

    // FIXME: this just hangs, I don't understand why.
    // It seems that the request is not sent
    println!("{:?}", client.add(&vec![1,2,3]).wait());
    println!("{:?}", client.sub(&vec![1]).wait());
    println!("{:?}", client.res().wait());
    println!("{:?}", client.clear().wait());

    // This, on the other hand, works.
    //
    // use tokio_core::reactor::Core;
    // let mut core = Core::new().unwrap();
    // let handle = core.handle();
    // core.run(
    //     rmp_rpc::Client::connect(&addr, &handle).and_then(|client| {
    //         client.request("res", vec![]).and_then(move |response| {
    //             println!("client: {:?}", response);
    //             Ok(())
    //         })
    //     })).unwrap();
}
