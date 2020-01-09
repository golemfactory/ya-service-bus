/// Using GSB with actix 0.8
use super::error::Error as BusError;
use super::Handle;
use crate::local_router::{router, Router};
use crate::{RpcEnvelope, RpcMessage};
use actix::prelude::*;

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::{Arc, Mutex};

pub fn bind<M: RpcMessage>(addr: &str, actor: Recipient<RpcEnvelope<M>>) -> Handle
where
    <RpcEnvelope<M> as Message>::Result: Serialize + DeserializeOwned + Sync + Send,
{
    router().lock().unwrap().bind_actor(addr, actor);
    Handle { _inner: {} }
}

pub fn service(addr: &str) -> Endpoint {
    Endpoint {
        addr: addr.into(),
        router: router(),
    }
}

pub struct Endpoint {
    addr: String,
    router: Arc<Mutex<Router>>,
}

impl Endpoint {
    pub fn send<M: RpcMessage + Serialize + DeserializeOwned + Sync + Send + Unpin>(
        &self,
        msg: M,
    ) -> impl Future<Output = Result<<RpcEnvelope<M> as Message>::Result, BusError>> + Unpin + 'static
    {
        let mut b = self.router.lock().unwrap();
        b.forward(self.addr.as_ref(), msg)
    }
}
