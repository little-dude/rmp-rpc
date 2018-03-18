//! Here is an simple example with a pure client. A "pure" client does not handle incoming requests
//! or notifications, and can only send requests and notifications, and handle responses to
//! requests it sends). `rmp-rpc` also makes it possible to have a client that also act as a server
//! and handles incoming requests and notifications.
extern crate env_logger;
extern crate futures;
extern crate rmp_rpc;
extern crate tokio_core;

use std::net::SocketAddr;

use futures::Future;
use rmp_rpc::Client;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;

fn main() {
    env_logger::init().unwrap();

    // Create a new tokio event loop to run the client
    let mut core = Core::new().unwrap();

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let handle = core.handle();

    // Create a future that connects to the server, and send a notification and a request.
    let client = TcpStream::connect(&addr, &handle)
        .or_else(|e| {
            println!("I/O error in the client: {}", e);
            Err(())
        })
        .and_then(move |stream| {
            let client = Client::new(stream, &handle);

            // Use the client to send a notification.
            // The future returned by client.notify() finishes when the notification
            // has been sent, in case we care about that. We can also just drop it.
            client.notify("hello", &[]);

            // Use the client to send a request with the method "dostuff", and two parameters:
            // the string "foo" and the integer "42".
            // The future returned by client.request() finishes when the response
            // is received.
            client
                .request("dostuff", &["foo".into(), 42.into()])
                .and_then(|response| {
                    println!("Response: {:?}", response);
                    Ok(())
                })
        });

    // Run the client
    match core.run(client) {
        Ok(_) => println!("Client finished successfully"),
        Err(_) => println!("Client failed"),
    }
}
