[![Build Status](https://travis-ci.org/little-dude/rmp-rpc.svg?branch=master)](https://travis-ci.org/little-dude/rmp-rpc)
[![Documentation](https://docs.rs/rmp-rpc/badge.svg)](https://docs.rs/crate/rmp-rpc)
[![crates.io](https://img.shields.io/crates/v/rmp-rpc.svg)](https://crates.io/crates/rmp-rpc)

rmp-rpc
=======

A Rust implementation of MessagePack-RPC based on [tokio](http://tokio.rs/).

Features
========

- [X] Support all the features described in [MessagePack-RPC specifications](https://github.com/msgpack/msgpack/blob/master/spec.md).
- [ ] Transport:
    - [X] TCP
    - [X] TLS over TCP
    - [ ] HTTP
    - [ ] stdin/stdout
- [X] Support for endpoints that act both as client and server. This is not part of the specification, but is a relatively common use of MessagePack-RPC.

Examples
========

    - [client.rs](examples/client.rs): a simple client
    - [server.rs](examples/server.rs): a simple server
    - [Calculator](examples/calculator.rs): a calculator application: the server performs simple arithmetic operations (addition, substraction) and returns the results to the client.
    - [Ping Pong](examples/ping_pong.rs): an example with endpoints that are both client and server.
