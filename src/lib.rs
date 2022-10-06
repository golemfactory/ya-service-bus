use actix::Message;
use futures::prelude::Stream;
use serde::{de::DeserializeOwned, Serialize};
use std::{fmt::Debug, future::Future};

pub mod actix_rpc;
pub mod connection;
pub mod error;
mod local_router;
mod remote_router;
pub mod serialization;
pub mod timeout;
pub mod typed;
pub mod untyped;

pub use error::Error;

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

#[derive(Debug)]
pub struct RpcStreamCall<T: RpcStreamMessage> {
    pub caller: String,
    pub addr: String,
    pub body: T,
    pub reply: futures::channel::mpsc::Sender<Result<T::Item, T::Error>>,
}

// Represents raw response chunk
pub enum ResponseChunk {
    Part(Vec<u8>),
    Full(Vec<u8>),
}

impl ResponseChunk {
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            ResponseChunk::Part(data) => data,
            ResponseChunk::Full(data) => data,
        }
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        match self {
            ResponseChunk::Full(_) => true,
            _ => false,
        }
    }

    pub fn is_eos(&self) -> bool {
        match self {
            ResponseChunk::Full(data) => data.is_empty(),
            _ => false,
        }
    }
}

pub struct RpcRawStreamCall {
    pub caller: String,
    pub addr: String,
    pub body: Vec<u8>,
    pub reply: futures::channel::mpsc::Sender<Result<ResponseChunk, error::Error>>,
}

impl Message for RpcRawStreamCall {
    type Result = Result<(), error::Error>;
}

pub struct RpcRawCall {
    pub caller: String,
    pub addr: String,
    pub body: Vec<u8>,
    pub no_reply: bool,
}

impl RpcRawCall {
    fn from_envelope_addr<T: Serialize>(
        envelope: RpcEnvelope<T>,
        addr: String,
        no_reply: bool,
    ) -> Self {
        RpcRawCall {
            caller: envelope.caller,
            addr,
            body: crate::serialization::to_vec(&envelope.body).unwrap(),
            no_reply,
        }
    }
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

    pub fn with_caller(caller: impl ToString, body: T) -> Self {
        RpcEnvelope {
            caller: caller.to_string(),
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

    fn send_as(&self, caller: impl ToString + 'static, msg: T) -> Self::Result;
}

pub trait RpcHandler<T: RpcMessage> {
    type Result: Future<Output = <RpcEnvelope<T> as Message>::Result> + 'static;

    fn handle(&mut self, caller: String, msg: T) -> Self::Result;
}

pub trait RpcStreamHandler<T: RpcStreamMessage> {
    type Result: Stream<Item = Result<T::Item, T::Error>> + Unpin;

    fn handle(&mut self, caller: &str, msg: T) -> Self::Result;
}

pub struct Handle {
    pub(crate) _inner: (),
}
