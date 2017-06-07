use tokio_proto::pipeline::{ClientProto};
use std::collections::HashMap;
use tokio_core::reactor::Handle;
use tokio_core::net::TcpStream;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::Framed;
use tokio_proto::{BindServer};
use tokio_service::Service;
use futures::future::{Loop, loop_fn};
use futures::{Future, Async, Sink, Stream};
use message::Message;
use codec::Codec;
use std::io;

struct ServerState<S: Service> {
    frames: Framed<TcpStream, Codec>,
    service:  S,
    request_tasks: HashMap<u32, Box<Future<Item=Option<Message>, Error=io::Error>>>,
    notification_tasks: Vec<Box<Future<Item=Option<Message>, Error=io::Error>>>,
}

// A dummy struct. Now sure why, but BindServer need a `Kind` type.
struct MsgpackRpc;

pub struct ServerProto;

impl BindServer<MsgpackRpc, TcpStream> for ServerProto
{
    type ServiceRequest = Message;
    type ServiceResponse = Option<Message>;
    type ServiceError = io::Error;

    fn bind_server<S>(&self, handle: &Handle, io: TcpStream, service: S)
        where S: Service<Request = Self::ServiceRequest,
                         Response = Self::ServiceResponse,
                         Error = Self::ServiceError> + 'static
    {
        let server_state = ServerState {
            frames: io.framed(Codec),
            service: service,
            request_tasks: HashMap::new(),
            notification_tasks: vec![],
        };

        let handle_messages = loop_fn(
            server_state,
            |mut server_state| {

                // For now, just read all the requests from the socket, and panic when we get one.
                loop {
                    match server_state.frames.poll().unwrap() {
                        Async::Ready(Some(request)) => {
                            // XXX: why is this never reached?
                            panic!("Got a request! I can die happy now");
                            match request {
                                Message::Request { id, .. } => {
                                    let response = server_state.service.call(request);
                                    server_state.request_tasks.insert(id, Box::new(response));
                                }
                                Message::Notification { .. } => {
                                    let response = server_state.service.call(request);
                                    server_state.notification_tasks.push(Box::new(response));
                                }
                                Message::Response { .. }=> {
                                    // TODO: we do expect responses
                                    panic!("Did not expect to receive a response");
                                }
                            };
                        }
                        Async::Ready(None) => {
                            // TODO: shutdown gracefully
                            break;
                        }
                        Async::NotReady =>  {
                            break;
                        }
                    }
                }

                // Process responses. We start with notifications.
                {
                    let mut finished_tasks = vec![];
                    for (idx, task) in server_state.notification_tasks.iter_mut().enumerate() {
                        match task.poll().unwrap() {
                            Async::Ready(Some(_)) => {
                                panic!("Notifications are not supposed to return anything");
                            }
                            Async::Ready(None) => {
                                // The future has finished
                                finished_tasks.push(idx);
                            }
                            Async::NotReady => {
                                // The future has not finished yet, we ignore it
                                continue;
                            }
                        }
                    }
                    for idx in finished_tasks.iter().rev() {
                        server_state.notification_tasks.remove(*idx);
                    }
                }

                // Process requests
                {
                    let mut finished_tasks = vec![];
                    for (id, task) in server_state.request_tasks.iter_mut() {
                        match task.poll().unwrap() {

                            // the request has been processed and a response is available.
                            Async::Ready(Some(response)) => {
                                // FIXME: set the response id
                                // response.id = id;

                                // send the response
                                server_state.frames.start_send(response).unwrap();
                                // remember this task is done so that we can clean it up after
                                finished_tasks.push(*id);
                            }
                            Async::Ready(None) => {
                                panic!("A response should be available");
                            }
                            Async::NotReady => {
                                // The future has not finished yet, we ignore it
                                continue;
                            }
                        }
                    }

                    let _ = server_state.frames.poll_complete();

                    for idx in finished_tasks.iter_mut().rev() {
                        let _ = server_state.request_tasks.remove(idx);
                    }
                }

                Ok(Loop::Continue(server_state))
            });

        handle.spawn(handle_messages)
    }
}

pub struct Protocol;

// impl<T: AsyncRead + AsyncWrite + 'static> ServerProto<T> for Protocol {
//     type Request = Message;
//     type Response = Message;
//     type Transport = Framed<T, Codec>;
//     type BindTransport = Result<Self::Transport, io::Error>;
//     fn bind_transport(&self, io: T) -> Self::BindTransport {
//         Ok(io.framed(Codec))
//     }
// }

impl<T: AsyncRead + AsyncWrite + 'static> ClientProto<T> for Protocol {
    type Request = Message;
    type Response = Message;
    type Transport = Framed<T, Codec>;
    type BindTransport = Result<Self::Transport, io::Error>;
    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(Codec))
    }
}
