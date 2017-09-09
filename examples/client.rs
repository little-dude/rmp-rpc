//! Here is an simple example with a pure client. A "pure" client does not handle incoming requests
//! or notifications, and can only send requests and notifications, and handle responses to
//! requests it sends). `rmp-rpc` also makes it possible to have a client that also act as a server
//! and handles incoming requests and notifications.
extern crate futures;
extern crate rmp_rpc;
extern crate tokio_core;

use std::net::SocketAddr;

use futures::Future;
use rmp_rpc::ClientOnlyConnector;
use tokio_core::reactor::Core;

fn main() {
    // Create a new tokio event loop to run the client
    let mut core = Core::new().unwrap();

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let handle = core.handle();

    // Create a future that connects to the server, and send a notification and a request.
    let client = ClientOnlyConnector::new(&addr, &handle)
        .connect()
        .or_else(|e| {
            println!("Connection to server failed: {}", e);
            Err(())
        })
        .and_then(|client| {
            // send a notification with the method "hello" and no argument
            // The future returned by client.notify() finishes when the notification
            // has been sent.
            client.notify("hello", &[]).and_then(|_| {
                // The notification has been sent.
                // We return the client so that we can chain this future
                Ok(client)
            })
        })
        .and_then(|client| {
            // send a request with the method "dostuff", and two parameter:
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
        Ok(()) => println!("Client finished successfully"),
        Err(()) => println!("Client failed"),
    }
}
