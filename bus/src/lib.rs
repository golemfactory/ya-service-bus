use actix::Message;
use futures::prelude::Stream;
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

pub mod actix_rpc;
mod error;
mod local_router;

pub trait BusMessage {}

pub trait RpcMessage: Serialize + DeserializeOwned + 'static + Sync + Send {
    const ID: &'static str;
    type Item: Serialize + DeserializeOwned + 'static + Sync + Send;
    type Error: Serialize + DeserializeOwned + 'static + Sync + Send + Debug;
}

pub struct RpcEnvelope<T: RpcMessage> {
    caller: String,
    body: T,
}

impl<T: RpcMessage> RpcEnvelope<T> {
    pub fn into_inner(self) -> T {
        self.body
    }

    pub fn with_caller(caller: impl Into<String>, body: T) -> Self {
        RpcEnvelope {
            caller: caller.into(),
            body,
        }
    }

    pub fn local(body: T) -> Self {
        RpcEnvelope {
            caller: "local".into(),
            body,
        }
    }

    pub fn caller(&self) -> &str {
        self.caller.as_str()
    }
}

impl<T: RpcMessage> Message for RpcEnvelope<T> {
    type Result = Result<T::Item, T::Error>;
}

impl<T: RpcMessage> AsRef<T> for RpcEnvelope<T> {
    fn as_ref(&self) -> &T {
        &self.body
    }
}

impl<T: RpcMessage> AsMut<T> for RpcEnvelope<T> {
    fn as_mut(&mut self) -> &mut T {
        &mut self.body
    }
}

impl<T: RpcMessage> std::ops::Deref for RpcEnvelope<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

impl<T: RpcMessage> std::ops::DerefMut for RpcEnvelope<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.body
    }
}

pub trait RpcEndpoint<T: RpcMessage>: Clone {
    type Result: Future<Output = Result<<RpcEnvelope<T> as Message>::Result, error::Error>>;

    fn send(&self, msg: T) -> Self::Result;
}

pub trait RpcHandler<T: RpcMessage> {
    type Result: Future<Output = <RpcEnvelope<T> as Message>::Result> + 'static;

    fn handle(&mut self, caller: &str, msg: T) -> Self::Result;
}

pub trait RpcStreamHandler<T: RpcMessage> {
    type Result: Stream<Item = <RpcEnvelope<T> as Message>::Result>;

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

pub fn send_untyped(
    addr: &str,
    bytes: &[u8],
) -> impl Future<Output = Result<Vec<u8>, error::Error>> {
    local_router::router()
        .lock()
        .unwrap()
        .forward_bytes(addr, "local", bytes)
}

struct MockEndpoint<T>(PhantomData<T>);

impl<T> Clone for MockEndpoint<T> {
    fn clone(&self) -> Self {
        MockEndpoint(PhantomData::default())
    }
}

impl<M: RpcMessage> RpcEndpoint<M> for MockEndpoint<M> {
    type Result = futures::future::Ready<Result<<RpcEnvelope<M> as Message>::Result, error::Error>>;

    fn send(&self, msg: M) -> Self::Result {
        unimplemented!()
    }
}
