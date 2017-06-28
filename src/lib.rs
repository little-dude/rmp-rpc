//! This crate provides facilities to use the `MessagePack` remote procedure call system
//! (`MessagePack-RPC`) in Rust.

#![cfg_attr(feature="clippy", feature(plugin))]
#![cfg_attr(feature="clippy", plugin(clippy))]
#![cfg_attr(feature="clippy", deny(clippy))]
#![cfg_attr(feature="clippy", allow(missing_docs_in_private_items))]

extern crate rmpv;
extern crate rmp;
extern crate bytes;
extern crate futures;
extern crate tokio_io;
extern crate tokio_core;

mod errors;
mod codec;
mod message;
mod server;
mod client;

pub use client::ClientProxy as Client;
pub use client::Connection;
pub use server::{serve, Service, ServiceBuilder};
pub use message::{Request, Response, Notification, Message};

/// Re-exports from [rmpv](https://docs.rs/rmpv)
pub use rmpv::{Value, Integer, Utf8String};
