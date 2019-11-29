use actix::Message;
use failure::_core::marker::PhantomData;
use futures::prelude::Stream;
use serde::{de::DeserializeOwned, Serialize};
use std::future::Future;
use std::sync::Arc;

pub mod actix_rpc;
mod error;
mod local_router;

pub trait BusMessage {}

pub trait RpcMessage: Serialize + DeserializeOwned + 'static + Sync + Send {
    const ID: &'static str;
    type Item: Serialize + DeserializeOwned + 'static + Sync + Send;
    type Error: Serialize + DeserializeOwned + 'static + Sync + Send;
}

impl<T: RpcMessage + Sized> Message for T {
    type Result = Result<T::Item, T::Error>;
}

pub trait RpcEndpoint<T: RpcMessage>: Clone {
    type Result: Future<Output = Result<<T as Message>::Result, error::Error>>;

    fn send(&self, msg: T) -> Self::Result;
}

pub trait RpcHandler<T: RpcMessage> {
    type Result: Future<Output = <T as Message>::Result> + 'static;

    fn handle(&mut self, caller: &str, msg: T) -> Self::Result;
}

pub trait RpcStreamHandler<T: RpcMessage> {
    type Result: Stream<Item = <T as Message>::Result>;

    fn handle(&mut self, caller: &str, msgs: Vec<T>) -> Self::Result;
}

pub struct Handle {
    pub(crate) _inner: (),
}

pub fn bind<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T>) -> Handle {
    unimplemented!()
}

pub fn service<T: RpcMessage>(addr: &str) -> impl RpcEndpoint<T> {
    MockEndpoint(PhantomData::default())
}

struct MockEndpoint<T>(PhantomData<T>);

impl<T> Clone for MockEndpoint<T> {
    fn clone(&self) -> Self {
        MockEndpoint(PhantomData::default())
    }
}

impl<M: RpcMessage> RpcEndpoint<M> for MockEndpoint<M> {
    type Result = futures::future::Ready<Result<<M as Message>::Result, error::Error>>;

    fn send(&self, msg: M) -> Self::Result {
        unimplemented!()
    }
}
