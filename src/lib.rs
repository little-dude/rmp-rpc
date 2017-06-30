//! This crate provides facilities to use the `MessagePack` remote procedure call system
//! (`MessagePack-RPC`) in Rust.

#![cfg_attr(feature="clippy", feature(plugin))]
#![cfg_attr(feature="clippy", plugin(clippy))]
#![cfg_attr(feature="clippy", deny(clippy))]
#![cfg_attr(feature="clippy", allow(missing_docs_in_private_items))]

extern crate rmp;
extern crate rmpv;
extern crate bytes;
extern crate futures;
#[macro_use]
extern crate log;
extern crate tokio_io;
extern crate tokio_core;

mod errors;
mod codec;
mod message;
pub mod server;
pub mod client;

pub use message::{Request, Response, Notification, Message};
pub use rmpv::{Value, Integer, Utf8String};
