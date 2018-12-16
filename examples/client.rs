//! Here is an simple example with a pure client. A "pure" client does not handle incoming requests
//! or notifications, and can only send requests and notifications, and handle responses to
//! requests it sends). `rmp-rpc` also makes it possible to have a client that also act as a server
//! and handles incoming requests and notifications.
use env_logger;


use tokio;

use std::net::SocketAddr;

use futures::Future;
use rmp_rpc::Client;
use tokio::net::TcpStream;

fn main() {
    env_logger::init();

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();

    // Create a future that connects to the server, and send a notification and a request.
    let client = TcpStream::connect(&addr)
        .or_else(|e| {
            println!("I/O error in the client: {}", e);
            Err(())
        })
        .and_then(move |stream| {
            let client = Client::new(stream);

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
    tokio::run(client.then(|result| {
        match result {
            Ok(_) => println!("Client finished successfully"),
            Err(_) => println!("Client failed"),
        }
        result
    }));
}
