use actix::Message;
use futures::prelude::Stream;
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

pub mod actix_rpc;
pub mod connection;
mod error;
mod local_router;
mod remote_router;
pub mod typed;
pub mod untyped;

pub use error::Error;
use futures::TryStream;
use rmp_serde::encode::UnderlyingWrite;

pub trait RpcMessage: Serialize + DeserializeOwned + 'static + Sync + Send {
    const ID: &'static str;
    type Item: Serialize + DeserializeOwned + 'static + Sync + Send;
    type Error: Serialize + DeserializeOwned + 'static + Sync + Send + Debug;
}

pub trait RpcStreamMessage: Serialize + DeserializeOwned + 'static + Sync + Send {
    const ID: &'static str;
    type Item: Serialize + DeserializeOwned + 'static + Sync + Send;
    type Error: Serialize + DeserializeOwned + 'static + Sync + Send + Debug;
}

pub struct RpcEnvelope<T> {
    caller: String,
    body: T,
}

pub struct RpcStreamCall<T: RpcStreamMessage> {
    pub caller: String,
    pub addr: String,
    pub body: T,
    pub reply: futures_01::sync::mpsc::Sender<Result<T::Item, T::Error>>,
}

// Represents raw response chunk
pub enum ResponseChunk {
    Part(Vec<u8>),
    Full(Vec<u8>),
}

pub struct RpcRawStreamCall {
    pub caller: String,
    pub addr: String,
    pub body: Vec<u8>,
    pub reply: futures_01::sync::mpsc::Sender<ResponseChunk>,
}

impl Message for RpcRawStreamCall {
    type Result = Result<(), error::Error>;
}

pub struct RpcRawCall {
    pub caller: String,
    pub addr: String,
    pub body: Vec<u8>,
}

impl Message for RpcRawCall {
    type Result = Result<Vec<u8>, error::Error>;
}

impl<T: RpcStreamMessage> Message for RpcStreamCall<T> {
    type Result = Result<(), error::Error>;
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

pub trait RpcStreamHandler<T: RpcStreamMessage> {
    type Result: TryStream<Ok = T::Item, Error = T::Error> + Unpin;

    fn handle(&mut self, caller: &str, msg: T) -> Self::Result;
}

pub struct Handle {
    pub(crate) _inner: (),
}
