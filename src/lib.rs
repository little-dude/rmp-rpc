//! This crate provides facilities to use the `MessagePack` remote procedure call system
//! (`msgpack-RPC`) in Rust.

#![cfg_attr(feature="clippy", feature(plugin))]
#![cfg_attr(feature="clippy", plugin(clippy))]
#![cfg_attr(feature="clippy", deny(clippy))]
#![cfg_attr(feature="clippy", allow(missing_docs_in_private_items))]

extern crate rmpv;
extern crate rmp;
extern crate bytes;
extern crate futures;
extern crate futures_cpupool;
extern crate tokio_io;
extern crate tokio_proto;
extern crate tokio_service;

pub mod errors;
pub mod message;
pub mod codec;
pub mod server;
