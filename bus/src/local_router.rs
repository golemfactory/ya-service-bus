use crate::{
    error::Error,
    remote_router::{RemoteRouter, UpdateService},
    Handle, RpcEnvelope, RpcHandler, RpcMessage, RpcRawCall,
};
use actix::prelude::*;
use futures::prelude::*;
use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use std::pin::Pin;
use ya_sb_util::PrefixLookupBag;

mod into_actix;

trait RawEndpoint: Any {
    fn send(&self, msg: RpcRawCall) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>>;

    fn recipient(&self) -> &dyn Any;
}

impl<T: RpcMessage> RawEndpoint for Recipient<RpcEnvelope<T>> {
    fn send(&self, msg: RpcRawCall) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>> {
        let body: T = rmp_serde::decode::from_read(msg.body.as_slice()).unwrap();
        Box::pin(
            Recipient::send(self, RpcEnvelope::with_caller(&msg.caller, body))
                .map_err(|e| e.into())
                .and_then(|r| async { rmp_serde::to_vec(&r).map_err(Error::from) }),
        )
    }

    fn recipient(&self) -> &dyn Any {
        self
    }
}

impl RawEndpoint for Recipient<RpcRawCall> {
    fn send(&self, msg: RpcRawCall) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>> {
        Box::pin(
            Recipient::<RpcRawCall>::send(self, msg)
                .map_err(Error::from)
                .then(|v| async { v? }),
        )
    }

    fn recipient(&self) -> &dyn Any {
        self
    }
}

struct Slot {
    inner: Box<dyn RawEndpoint + Send + 'static>,
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

    fn from_raw(r: Recipient<RpcRawCall>) -> Self {
        Slot { inner: Box::new(r) }
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

    fn send(&self, msg: RpcRawCall) -> impl Future<Output = Result<Vec<u8>, Error>> {
        self.inner.send(msg)
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

    pub fn bind<T: RpcMessage>(
        &mut self,
        addr: &str,
        endpoint: impl RpcHandler<T> + 'static,
    ) -> Handle {
        let slot = Slot::from_handler(endpoint);
        let addr = format!("{}/{}", addr, T::ID);
        let _ = self.handlers.insert(addr.clone(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr.into()));
        Handle { _inner: () }
    }

    pub fn bind_actor<T: RpcMessage>(&mut self, addr: &str, endpoint: Recipient<RpcEnvelope<T>>) {
        let slot = Slot::from_actor(endpoint);
        let addr = format!("{}/{}", addr, T::ID);
        let _ = self.handlers.insert(addr.clone(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr));
    }

    pub fn bind_raw(&mut self, addr: &str, endpoint: Recipient<RpcRawCall>) -> Handle {
        let slot = Slot::from_raw(endpoint);
        let _ = self.handlers.insert(addr.to_string(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr.into()));
        Handle { _inner: () }
    }

    pub fn forward<T: RpcMessage>(
        &mut self,
        addr: &str,
        msg: T,
    ) -> impl Future<Output = Result<Result<T::Item, T::Error>, Error>> {
        let caller = "local";
        let addr = format!("{}/{}", addr, T::ID);
        if let Some(slot) = self.handlers.get_mut(&addr) {
            (if let Some(h) = slot.recipient() {
                h.send(RpcEnvelope::local(msg))
                    .map_err(Error::from)
                    .left_future()
            } else {
                let body = rmp_serde::to_vec(&msg).unwrap();
                slot.send(RpcRawCall {
                    caller: caller.into(),
                    addr,
                    body,
                })
                .and_then(|b| async { Ok(rmp_serde::from_read_ref(&b)?) })
                .right_future()
            })
            .left_future()
        } else {
            let body = rmp_serde::to_vec(&msg).unwrap();

            RemoteRouter::from_registry()
                .send(RpcRawCall {
                    caller: caller.into(),
                    addr,
                    body,
                })
                .then(|v| async { Ok(v?) })
                .and_then(|b| async { Ok(rmp_serde::from_read_ref(&b?)?) })
                .right_future()
        }
    }

    pub fn forward_bytes(
        &mut self,
        addr: &str,
        from: &str,
        msg: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, Error>> {
        if let Some(slot) = self.handlers.get_mut(addr) {
            slot.send(RpcRawCall {
                caller: from.into(),
                addr: addr.into(),
                body: msg,
            })
            .left_future()
        } else {
            RemoteRouter::from_registry()
                .send(RpcRawCall {
                    caller: from.into(),
                    addr: addr.into(),
                    body: msg,
                })
                .then(|v| async { v? })
                .right_future()
        }
    }

    pub fn forward_bytes_local(
        &mut self,
        addr: &str,
        from: &str,
        msg: &[u8],
    ) -> impl Future<Output = Result<Vec<u8>, Error>> {
        if let Some(slot) = self.handlers.get_mut(addr) {
            slot.send(RpcRawCall {
                caller: from.into(),
                addr: addr.into(),
                body: msg.into(),
            })
            .left_future()
        } else {
            log::warn!("no endpoint: {}", addr);
            future::err(Error::NoEndpoint).right_future()
        }
    }
}

lazy_static::lazy_static! {
static ref ROUTER: Arc<Mutex<Router>> = Arc::new(Mutex::new(Router::new()));
}

pub fn router() -> Arc<Mutex<Router>> {
    ROUTER.clone()
}
