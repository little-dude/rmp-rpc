use std::io;
use tokio_service::Service;
use futures::{Future, BoxFuture};
use message::{Message, Response};
use rmpv::Value;
use futures_cpupool::CpuPool;
use std::marker::Sync;

// FIXME: The 'static bound is quite limiting because it means that we can implement Dispatch for
// types like Foo<'a>. It is required because in Service.call(), we move a dispatcher into a
// closure, and it has to live long enough inside this closure, i.e. at least as long as the
// closure lives.
//
// Is this even fixable?
/// A dispatcher that performs the calls on the server.
///
/// # Examples
///
/// Here is how to implement a simple server with two methods `hello` (that returns "hello") and
/// `world` (that return "world").  Calling any other method would result in an error.
///
/// ```
/// extern crate rmpv;
/// extern crate rmp_rpc;
///
/// use rmpv::{Value, Utf8String};
/// use rmp_rpc::Dispatch;
///
/// #[derive(Clone)]
/// pub struct HelloWorld;
///
/// impl Dispatch for HelloWorld {
///     fn dispatch(&self, method: &str, _params: &[Value]) -> Result<Value, Value> {
///         match method {
///             "hello" => { Ok(Value::String(Utf8String::from("hello"))) }
///             "world" => { Ok(Value::String(Utf8String::from("world"))) }
///             _ => { Err(Value::String(Utf8String::from(format!("Invalid method {}", method)))) }
///         }
///     }
/// }
///
/// # fn main() {}
///
/// ```
///
pub trait Dispatch: Send + Sync + Clone + 'static {
    /// Respond a request. `method` is the name of the `MessagePack-RPC` method that was called, and
    /// `params` its arguments.
    fn dispatch(&self, method: &str, params: &[Value]) -> Result<Value, Value>;
}

/// A `MessagePack-RPC` server. It calls a dispatcher to answer requests.
pub struct Server<T: Dispatch> {
    dispatcher: T,
    thread_pool: CpuPool,
}

impl<T: Dispatch> Server<T> {
    /// Instantiate a new server based on a given dispatcher.
    pub fn new(dispatcher: T) -> Self {
        Server {
            dispatcher: dispatcher,
            thread_pool: CpuPool::new(10),
        }
    }
}

impl<T: Dispatch> Service for Server<T> {
    type Request = Message;
    type Response = Message;
    type Error = io::Error;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn call(&self, message: Self::Request) -> Self::Future {
        match message {
            Message::Request(request) => {
                // FIXME: The whole dispatcher is cloned for every request won't that kill
                // performances ? How could we avoid that?
                let dispatcher = self.dispatcher.clone();

                // FIXME: is that how we are supposed to create futures?
                // `self.thread_pool::spawn_fn` will create a new thread for each request, that
                // seems overkill..
                let future = self.thread_pool.spawn_fn(move || {
                    Ok(Message::Response(Response {
                        result: dispatcher.dispatch(request.method.as_str(), &request.params),
                        id: 0,
                    }))
                });

                future.boxed()
            }
            // TODO: Notifications
            _ => unimplemented!(),
        }
    }
}
