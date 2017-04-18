extern crate futures;
extern crate tokio_core;
extern crate tokio_proto;
extern crate rmp_rpc;

mod client;
mod server;

use client::connect;
use server::Calculator;
use tokio_proto::TcpServer;
use rmp_rpc::{Protocol, Server};
use futures::Future;
use std::thread;
use std::time::Duration;

fn main() {

    let addr = "127.0.0.1:54321".parse().unwrap();

    thread::spawn(move || {
        let tcp_server = TcpServer::new(Protocol, addr);
        tcp_server.serve(|| Ok(Server::new(Calculator::new())));
    });

    thread::sleep(Duration::from_millis(100));

    let client = connect(&addr).expect("Connection failed");

    // FIXME: this just hangs, I don't understand why.
    // It seems that the request is not sent
    println!("{:?}", client.add(&vec![1, 2, 3]).wait());
    println!("{:?}", client.sub(&vec![1]).wait());
    println!("{:?}", client.res().wait());
    println!("{:?}", client.clear().wait());
}
