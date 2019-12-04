use crate::error::Error;
use crate::{RpcEnvelope, RpcHandler, RpcMessage};
use actix::Message;
use failure::_core::marker::PhantomData;
use failure::_core::pin::Pin;
use futures::channel::mpsc;
use futures::future;
use futures::prelude::*;
use futures::{
    compat::Future01CompatExt,
    future::{FutureExt, TryFutureExt},
    Future,
};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
mod into_actix;
mod util;

use crate::local_router::into_actix::RpcHandlerWrapper;
use crate::local_router::util::PrefixLookupBag;
use actix::Actor;

trait RawEndpoint: Any {
    fn send(
        &mut self,
        addr: &str,
        from: &str,
        msg: &[u8],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>>;

    fn recipient(&self) -> &dyn Any;
}

impl<T: RpcMessage, H: RpcHandler<T> + 'static> RawEndpoint for RpcHandlerWrapper<T, H> {
    fn send(
        &mut self,
        addr: &str,
        from: &str,
        msg: &[u8],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>> {
        let mut a = || {
            let request_msg: T = match rmp_serde::from_read_ref(msg) {
                Ok(v) => v,
                Err(e) => return future::Either::Left(future::err(Error::from(e))),
            };
            future::Either::Right(
                self.0
                    .handle(from, request_msg)
                    .map(|response_msg| rmp_serde::to_vec(&response_msg).map_err(Error::from)),
            )
        };
        Box::pin(a())
    }

    fn recipient(&self) -> &dyn Any {
        self
    }
}

struct Slot {
    inner: Box<dyn RawEndpoint + 'static>,
}

impl Slot {
    fn from_handler<T: RpcMessage, H: RpcHandler<T> + 'static>(handler: H) -> Self {
        Slot {
            inner: Box::new(into_actix::RpcHandlerWrapper::new(handler)),
        }
    }

    fn recipient<T: RpcMessage>(&mut self) -> Option<actix::Recipient<RpcEnvelope<T>>>
    where
        <RpcEnvelope<T> as Message>::Result: Sync + Send + 'static,
    {
        if let Some(r) = self
            .inner
            .recipient()
            .downcast_ref::<actix::Recipient<RpcEnvelope<T>>>()
        {
            Some(r.clone())
        } else {
            None
        }
    }

    fn send(
        &mut self,
        addr: &str,
        from: &str,
        msg: &[u8],
    ) -> impl Future<Output = Result<Vec<u8>, Error>> {
        self.inner.send(addr, from, msg)
    }
}

struct Router {
    handlers: PrefixLookupBag<Slot>,
}

impl Router {
    pub fn bind<T: RpcMessage>(&mut self, addr: &str, endpoint: impl RpcHandler<T> + 'static) {
        let slot = Slot::from_handler(endpoint);
        let _ = self.handlers.insert(format!("{}/{}", addr, T::ID), slot);
    }

    pub async fn forward<T: RpcMessage>(
        &mut self,
        addr: &str,
        msg: T,
    ) -> Result<Result<T::Item, T::Error>, Error> {
        if let Some(slot) = self.handlers.get_mut(&format!("{}/{}", addr, T::ID)) {
            if let Some(h) = slot.recipient() {
                return Ok(h.send(RpcEnvelope::local(msg)).compat().await?);
            }
        }
        Err(Error::NoEndpoint)
    }

    pub async fn forward_bytes(
        &mut self,
        addr: &str,
        from: &str,
        msg: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if let Some(slot) = self.handlers.get_mut(addr) {
            slot.send(addr, from, msg).await
        } else {
            Err(Error::NoEndpoint)
        }
    }
}
