#![allow(unused)]

use super::error::Error as BusError;
use super::Handle;
use actix::prelude::*;
use futures_01::{future, Future};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub fn bind<M: Message + Serialize + DeserializeOwned + Sync + Send>(
    addr: &str,
    actor: Recipient<M>,
) -> Handle
where
    M::Result: Serialize + DeserializeOwned + Sync + Send,
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
    pub fn send<M: Message + Serialize + DeserializeOwned + Sync + Send>(
        &self,
        msg: M,
    ) -> impl Future<Item = M::Result, Error = BusError> {
        future::err(BusError::Closed)
    }
}
