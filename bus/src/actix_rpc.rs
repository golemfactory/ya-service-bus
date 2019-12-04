#![allow(unused)]

use super::error::Error as BusError;
use super::Handle;
use crate::{RpcEnvelope, RpcMessage};
use actix::prelude::*;
use futures_01::{future, Future};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub fn bind<M: RpcMessage>(addr: &str, actor: Recipient<RpcEnvelope<M>>) -> Handle
where
    <RpcEnvelope<M> as Message>::Result: Serialize + DeserializeOwned + Sync + Send,
{
    eprintln!("bind {} for {}", addr, std::any::type_name::<M>());
    Handle { _inner: {} }
}

pub fn service(addr: &str) -> Endpoint {
    Endpoint { _inner: () }
}

pub struct Endpoint {
    _inner: (),
}

impl Endpoint {
    pub fn send<M: RpcMessage + Serialize + DeserializeOwned + Sync + Send>(
        &self,
        msg: M,
    ) -> impl Future<Item = <RpcEnvelope<M> as Message>::Result, Error = BusError> {
        future::err(BusError::Closed)
    }
}
