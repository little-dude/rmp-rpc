//! This crate provides facilities to use the `MessagePack` remote procedure call system
//! (`MessagePack-RPC`) in Rust.

#[macro_use]
extern crate log;

mod codec;
mod endpoint;
mod errors;
pub mod message;

pub use crate::endpoint::{
    serve, Ack, Client, Endpoint, IntoStaticFuture, Response, Service, ServiceWithClient,
};
pub use rmpv::{Integer, Utf8String, Value};
