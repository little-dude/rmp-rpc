[![Build Status](https://travis-ci.org/little-dude/rmp-rpc.svg?branch=master)](https://travis-ci.org/little-dude/rmp-rpc)
[![Documentation](https://docs.rs/rmp-rpc/badge.svg)](https://docs.rs/crate/rmp-rpc)
[![crates.io](https://img.shields.io/crates/v/rmp-rpc.svg)](https://crates.io/crates/rmp-rpc)

rmp-rpc
=======

A Rust implementation of MessagePack-RPC based on [tokio](http://tokio.rs/).

Features
========

- [X] Support all the features described in [MessagePack-RPC specifications](https://github.com/msgpack/msgpack/blob/master/spec.md).
- [X] Support for endpoints that act both as client and server. This is not part of the specification, but is a relatively common use of MessagePack-RPC.
- [X] Support any transport layer.

Examples
========

- [client.rs](https://github.com/little-dude/rmp-rpc/blob/master/examples/client.rs): a simple client
- [server.rs](https://github.com/little-dude/rmp-rpc/blob/master/examples/server.rs): a simple server
- [Ping Pong](https://github.com/little-dude/rmp-rpc/blob/master/examples/ping_pong.rs): an example with endpoints that are both client and server.
