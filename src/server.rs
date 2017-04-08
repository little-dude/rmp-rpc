// use std::collections::BTreeMap;
use std::io;
use tokio_proto::pipeline::ServerProto;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::Framed;
use tokio_service::Service;
use futures::{Future, BoxFuture};
use codec::Codec;
use message::{Message, Response};
use rmpv::Value;
use futures_cpupool::CpuPool;
use std::marker::Sync;

pub struct Proto;

impl<T: AsyncRead + AsyncWrite + 'static> ServerProto<T> for Proto {
    type Request = Message;
    type Response = Message;
    type Transport = Framed<T, Codec>;
    type BindTransport = Result<Self::Transport, io::Error>;
    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(Codec))
    }
}

pub trait Serve: Send + Sync + Clone + 'static {
    fn dispatch(&self, method: &str, params: &[Value]) -> Result<Value, Value>;
}

pub struct Server<T: Serve> {
    server: T,
    thread_pool: CpuPool,
}

impl<T: Serve> Server<T> {
    pub fn new(server: T) -> Self {
        Server {
            server: server,
            thread_pool: CpuPool::new(10),
        }
    }
}

impl<T: Serve> Service for Server<T> {
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn call(&self, message: Self::Request) -> Self::Future {
        match message {
            Message::Request(request) => {
                let dispatcher = self.server.clone();
                let future = self.thread_pool.spawn_fn(move || {
                    match dispatcher.dispatch(request.method.as_str(), &request.params) {
                        Ok(value) => {
                            Ok(Message::Response(Response {
                                id: request.id,
                                result: Ok(value),
                            }))
                        }
                        Err(value) => {
                            Ok(Message::Response(Response {
                                id: request.id,
                                result: Err(value),
                            }))
                        }
                    }
                });

                future.boxed()
            }
            // TODO: Notifications
            _ => unimplemented!(),
        }
    }
}
