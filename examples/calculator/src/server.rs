use std::sync::{Arc, Mutex};
use std::io;

use futures::{future, BoxFuture};
use rmp_rpc::server::{ServiceBuilder, Service};
use rmpv::Value;

#[derive(Clone)]
pub struct Calculator {
    value: Arc<Mutex<i64>>,
}

impl Calculator {
    pub fn new() -> Self {
        Calculator { value: Arc::new(Mutex::new(0)) }
    }

    fn parse_args(params: &[Value]) -> Result<Vec<i64>, &'static str> {
        let mut ret = vec![];
        for value in params {
            let int = if let Value::Integer(int) = *value {
                int.as_i64().ok_or("Invalid arguments")?
            } else {
                return Err("Invalid arguments");
            };
            ret.push(int);
        }
        Ok(ret)
    }

    fn clear(&self) -> Result<i64, &'static str> {
        println!("server: clear");
        let mut value = self.value.lock().unwrap();
        *value = 0;
        Ok(*value)
    }

    fn res(&self) -> Result<i64, &'static str> {
        println!("server: res");
        Ok(*self.value.lock().unwrap())
    }

    fn add(&self, params: &[Value]) -> Result<i64, &'static str> {
        println!("server: add");
        let mut value = self.value.lock().unwrap();
        *value += Self::parse_args(params)?
            .iter()
            .fold(0, |acc, &int| acc + int);
        Ok(*value)
    }

    fn sub(&self, params: &[Value]) -> Result<i64, &'static str> {
        println!("server: sub");
        let mut value = self.value.lock().unwrap();
        *value -= Self::parse_args(params)?
            .iter()
            .fold(0, |acc, &int| acc + int);
        Ok(*value)
    }
}

impl Service for Calculator {
    type T = i64;
    type E = String;
    type Error = io::Error;

    fn handle_request(
        &mut self,
        method: &str,
        params: &[Value],
    ) -> BoxFuture<Result<Self::T, Self::E>, Self::Error> {
        let res = match method {
            "add" | "+" => self.add(params).map_err(|e| e.to_string()),
            "sub" | "-" => self.sub(params).map_err(|e| e.to_string()),
            "res" | "=" => self.res().map_err(|e| e.to_string()),
            "clear" => self.clear().map_err(|e| e.to_string()),
            method => Err(format!("Invalid method {}", method)),
        };
        Box::new(future::ok(res))
    }
    fn handle_notification(
        &mut self,
        _method: &str,
        _params: &[Value],
    ) -> BoxFuture<(), Self::Error> {
        unimplemented!();
    }
}

impl ServiceBuilder for Calculator {
    type Service = Calculator;

    fn build(&self) -> Self::Service {
        println!("server: calculator service called.");
        self.clone()
    }
}
