use crate::error::Error;
use crate::{RpcEnvelope, RpcHandler, RpcMessage};
use actix::{Message, Recipient};
use futures::channel::mpsc;
use futures::prelude::*;
use futures::{
    compat::Future01CompatExt,
    future::{FutureExt, TryFutureExt},
    Future,
};
use futures::{future, TryStreamExt};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::pin::Pin;
mod into_actix;
mod util;

use crate::local_router::into_actix::RpcHandlerWrapper;
use crate::local_router::util::PrefixLookupBag;
use actix::Actor;
use futures::future::ErrInto;
use futures_01::future::Future as _;
use std::sync::{Arc, Mutex};

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

impl<T: RpcMessage> RawEndpoint for Recipient<RpcEnvelope<T>> {
    fn send(
        &mut self,
        addr: &str,
        from: &str,
        msg: &[u8],
    ) -> Pin<Box<Future<Output = Result<Vec<u8>, Error>>>> {
        let msg: T = rmp_serde::decode::from_read(msg).unwrap();
        Recipient::send(self, RpcEnvelope::with_caller(from, msg))
            .map_err(|e| e.into())
            .and_then(|r| rmp_serde::to_vec(&r).map_err(Error::from))
            .compat()
            .boxed_local()
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
            inner: Box::new(
                into_actix::RpcHandlerWrapper::new(handler)
                    .start()
                    .recipient(),
            ),
        }
    }

    fn from_actor<T: RpcMessage>(r: Recipient<RpcEnvelope<T>>) -> Self {
        Slot { inner: Box::new(r) }
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

pub struct Router {
    handlers: PrefixLookupBag<Slot>,
}

impl Router {
    fn new() -> Self {
        Router {
            handlers: PrefixLookupBag::default(),
        }
    }

    pub fn bind<T: RpcMessage>(&mut self, addr: &str, endpoint: impl RpcHandler<T> + 'static) {
        let slot = Slot::from_handler(endpoint);
        let _ = self.handlers.insert(format!("{}/{}", addr, T::ID), slot);
    }

    pub fn bind_actor<T: RpcMessage>(&mut self, addr: &str, endpoint: Recipient<RpcEnvelope<T>>) {
        let slot = Slot::from_actor(endpoint);
        let _ = self.handlers.insert(format!("{}/{}", addr, T::ID), slot);
    }

    pub fn forward<T: RpcMessage>(
        &mut self,
        addr: &str,
        msg: T,
    ) -> impl futures_01::future::Future<Item = Result<T::Item, T::Error>, Error = Error> {
        eprintln!(
            "keys={:?}",
            self.handlers
                .keys()
                .map(|s| s.to_string())
                .collect::<Vec<String>>()
        );
        if let Some(slot) = self.handlers.get_mut(&format!("{}/{}", addr, T::ID)) {
            if let Some(h) = slot.recipient() {
                return futures_01::future::Either::A(
                    h.send(RpcEnvelope::local(msg)).map_err(Error::from),
                );
            }
        }
        futures_01::future::Either::B(futures_01::future::err(Error::NoEndpoint))
    }

    pub fn forward_bytes(
        &mut self,
        addr: &str,
        from: &str,
        msg: &[u8],
    ) -> impl Future<Output = Result<Vec<u8>, Error>> {
        if let Some(slot) = self.handlers.get_mut(addr) {
            future::Either::Left(slot.send(addr, from, msg))
        } else {
            future::Either::Right(future::ready(Err(Error::NoEndpoint)))
        }
    }
}

thread_local! {
    pub static ROUTER: Arc<Mutex<Router>> = Arc::new(Mutex::new(Router::new()));
}

pub fn router() -> Arc<Mutex<Router>> {
    ROUTER.with(|r| r.clone())
}
