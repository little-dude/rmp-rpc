//! Here is an simple example with a pure client. A "pure" client does not handle incoming requests
//! or notifications, and can only send requests and notifications, and handle responses to
//! requests it sends). `rmp-rpc` also makes it possible to have a client that also act as a server
//! and handles incoming requests and notifications.

use std::io;
use std::net::SocketAddr;

use rmp_rpc::Client;
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncReadCompatExt;

#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::init();

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();

    // Create a future that connects to the server, and send a notification and a request.
    let socket = TcpStream::connect(&addr).await?;
    let client = Client::new(socket.compat());

    // Use the client to send a notification.
    // The future returned by client.notify() finishes when the notification
    // has been sent, in case we care about that. We can also just drop it.
    client.notify("hello", &[]);

    // Use the client to send a request with the method "dostuff", and two parameters:
    // the string "foo" and the integer "42".
    // The future returned by client.request() finishes when the response
    // is received.
    match client.request("dostuff", &["foo".into(), 42.into()]).await {
        Ok(response) => println!("Response: {:?}", response),
        Err(e) => println!("Error: {:?}", e),
    }

    Ok(())
}
