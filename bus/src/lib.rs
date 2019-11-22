use futures::prelude::Stream;
use serde::{de::DeserializeOwned, Serialize};
use std::future::Future;

pub struct BusPath(Vec<String>);

impl From<&[&str]> for BusPath {
    fn from(path: &[&str]) -> Self {
        BusPath(path.into_iter().map(|&s| s.into()).collect())
    }
}

pub trait BusMessage: Clone + Serialize + DeserializeOwned + 'static + Sync + Send {}

pub trait RpcMessage: BusMessage {
    const ID: &'static str;
    type Reply: BusMessage;
}

pub trait RpcEndpoint<T: RpcMessage> {
    type Result: Future<Output = T::Reply>;

    fn send(&self, msg: T) -> Self::Result;
}

pub trait RpcHandler<T: RpcMessage> {
    type Result: Future<Output = T::Reply>;

    fn handle(&mut self, caller: BusPath, msg: T) -> Self::Result;
}

pub trait RpcStreamHandler<T: RpcMessage> {
    type Result: Stream<Item = T::Reply>;

    fn handle(&mut self, caller: BusPath, msgs: Vec<T>) -> Self::Result;
}

pub struct Handle;

pub fn bind<T: RpcMessage>(addr: &BusPath, endpoint: impl RpcHandler<T>) -> Handle {
    unimplemented!()
}

pub fn service<T: RpcMessage>(addr: &BusPath) -> impl RpcEndpoint<T> {
    MockEndpoint(unimplemented!())
}

struct MockEndpoint<T>(T);

impl<M: RpcMessage> RpcEndpoint<M> for MockEndpoint<M> {
    type Result = futures::future::Ready<M::Reply>;

    fn send(&self, msg: M) -> Self::Result {
        unimplemented!()
    }
}
