extern crate futures;
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate rmp_rpc;

mod client;
mod server;

use client::Client;
use server::Calculator;
use tokio_core::reactor::Core;
use futures::Future;
use std::thread;
use std::time::Duration;

fn main() {

    let addr = "127.0.0.1:54321".parse().unwrap();

    thread::spawn(move || rmp_rpc::server::serve(addr, Calculator::new()));

    thread::sleep(Duration::from_millis(100));

    let mut reactor = Core::new().expect("Failed to start even loop");
    let client_future = Client::connect(&addr, &reactor.handle())
        .and_then(|client| {
            println!("connected");
            client.add(&vec![1, 2, 3])
                .and_then(|result| {
                    println!("{}", result);
                    Ok(client)
                })
                .or_else(|rpc_err| {
                    println!("add failed: {}", rpc_err);
                    Err(rpc_err)
                })
        })
        .and_then(|client| {
            client.sub(&vec![1])
                .and_then(|result| {
                    println!("{}", result);
                    Ok(client)
                })
                .or_else(|rpc_err| {
                    println!("sub failed: {}", rpc_err);
                    Err(rpc_err)
                })
        })
        .and_then(|client| {
            client.res()
                .and_then(|result| {
                    println!("{}", result);
                    Ok(client)
                })
                .or_else(|rpc_err| {
                    println!("res failed: {}", rpc_err);
                    Err(rpc_err)
                })
        })
        .and_then(|client| {
            client.clear()
                .and_then(|result| {
                    println!("{}", result);
                    Ok(client)
                })
                .or_else(|rpc_err| {
                    println!("clear failed: {}", rpc_err);
                    Err(rpc_err)
                })
        });
    let _ = reactor.run(client_future).unwrap();
}
