use crate::error::Error;
use crate::{
    Handle, ResponseChunk, RpcEnvelope, RpcHandler, RpcMessage, RpcRawCall, RpcRawStreamCall,
    RpcStreamCall, RpcStreamHandler, RpcStreamMessage,
};
use actix::{Arbiter, MailboxError, Message, Recipient, WrapFuture};
use futures::channel::mpsc;
use futures::prelude::*;
use futures::{
    compat::Future01CompatExt,
    future::{FutureExt, TryFutureExt},
    AsyncReadExt, Future,
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
use crate::remote_router::{RemoteRouter, UpdateService};
use crate::untyped::RawHandler;
use crate::ResponseChunk::Full;
use actix::{Actor, SystemService};
use futures::future::ErrInto;
use futures_01::{future::Future as Future01, stream::Stream as Stream01};
use std::sync::{Arc, Mutex};

trait RawEndpoint: Any {
    fn send(&self, msg: RpcRawCall) -> Box<dyn Future01<Item = Vec<u8>, Error = Error>>;

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Box<dyn Stream01<Item = ResponseChunk, Error = Error>>;

    fn recipient(&self) -> &dyn Any;
}

// Implementation for non-streaming service
impl<T: RpcMessage> RawEndpoint for Recipient<RpcEnvelope<T>> {
    fn send(&self, msg: RpcRawCall) -> Box<dyn Future01<Item = Vec<u8>, Error = Error>> {
        let body: T = rmp_serde::decode::from_read(msg.body.as_slice()).unwrap();
        Box::new(
            Recipient::send(self, RpcEnvelope::with_caller(&msg.caller, body))
                .map_err(|e| e.into())
                .and_then(|r| rmp_serde::to_vec(&r).map_err(Error::from)),
        )
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Box<dyn Stream01<Item = ResponseChunk, Error = Error>> {
        let body: T = rmp_serde::decode::from_read(msg.body.as_slice()).unwrap();

        Box::new(
            Recipient::send(self, RpcEnvelope::with_caller(&msg.caller, body))
                .map_err(|e| e.into())
                .and_then(|r| rmp_serde::to_vec(&r).map_err(Error::from))
                .map(|v| ResponseChunk::Full(v))
                .into_stream(),
        )
    }

    fn recipient(&self) -> &dyn Any {
        self
    }
}

impl<T: RpcStreamMessage> RawEndpoint for Recipient<RpcStreamCall<T>> {
    fn send(&self, msg: RpcRawCall) -> Box<dyn Future01<Item = Vec<u8>, Error = Error>> {
        Box::new(futures_01::future::err(Error::GsbBadRequest(
            "non-streaming-request on streaming endpoint".into(),
        )))
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Box<dyn Stream01<Item = ResponseChunk, Error = Error>> {
        use futures_01::prelude::*;

        let body: T = rmp_serde::decode::from_read(msg.body.as_slice()).unwrap();
        let (tx, rx) = futures_01::sync::mpsc::channel(16);
        let (txe, rxe) = futures_01::sync::oneshot::channel();

        let call = RpcStreamCall {
            caller: msg.caller,
            addr: msg.addr,
            body,
            reply: tx,
        };
        let send_promise = Recipient::send(self, call).flatten();
        let recv_stream = rx
            .map_err(|()| Error::Closed)
            .then(|r| match r {
                Ok(v) => Ok(ResponseChunk::Part(
                    rmp_serde::to_vec(&v).map_err(Error::from)?,
                )),
                //Ok(Err(e)) => Err(e),
                Err(e) => Err(e),
            })
            .chain(rxe.flatten().into_stream());
        Arbiter::spawn(
            send_promise
                .then(move |v| {
                    txe.send(match v {
                        Ok(()) => Ok(ResponseChunk::Full(vec![])),
                        Err(e) => Err(e),
                    })
                })
                .map_err(|_| eprintln!("err")),
        );
        Box::new(recv_stream)
    }

    fn recipient(&self) -> &dyn Any {
        self
    }
}

impl RawEndpoint for Recipient<RpcRawCall> {
    fn send(&self, msg: RpcRawCall) -> Box<dyn Future01<Item = Vec<u8>, Error = Error>> {
        Box::new(
            Recipient::<RpcRawCall>::send(self, msg)
                .map_err(Error::from)
                .flatten(),
        )
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Box<dyn Stream01<Item = ResponseChunk, Error = Error>> {
        Box::new(
            Recipient::<RpcRawCall>::send(self, msg)
                .map_err(Error::from)
                .flatten()
                .map(|v| ResponseChunk::Full(v))
                .into_stream(),
        )
    }

    fn recipient(&self) -> &Any {
        self
    }
}

impl RawEndpoint for Recipient<RpcRawStreamCall> {
    fn send(&self, msg: RpcRawCall) -> Box<dyn Future01<Item = Vec<u8>, Error = Error>> {
        let (tx, rx) = futures_01::sync::mpsc::channel(1);
        // TODO: send error to caller
        Arbiter::spawn(
            self.send(RpcRawStreamCall {
                caller: msg.caller,
                addr: msg.addr,
                body: msg.body,
                reply: tx,
            })
            .flatten()
            .map_err(|e| eprintln!("cell error={}", e)),
        );
        Box::new(Future01::then(rx.into_future(), |h| match h {
            Ok((Some(ResponseChunk::Full(v)), _)) => Ok(v),
            _ => Err(Error::GsbBadRequest("partial response".into())),
        }))
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Box<dyn Stream01<Item = ResponseChunk, Error = Error>> {
        let (tx, rx) = futures_01::sync::mpsc::channel(16);
        // TODO: send error to caller
        Arbiter::spawn(
            self.send(RpcRawStreamCall {
                caller: msg.caller,
                addr: msg.addr,
                body: msg.body,
                reply: tx,
            })
            .flatten()
            .map_err(|e| eprintln!("cell error={}", e)),
        );
        Box::new(rx.map_err(|()| Error::Closed))
    }

    fn recipient(&self) -> &Any {
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

    fn from_stream_handler<T: RpcStreamMessage, H: RpcStreamHandler<T> + 'static>(
        handler: H,
    ) -> Self {
        Slot {
            inner: Box::new(
                into_actix::RpcStreamHandlerWrapper::new(handler)
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

    fn from_stream_actor<T: RpcStreamMessage>(r: Recipient<RpcStreamCall<T>>) -> Self {
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

    fn stream_recipient<T: RpcStreamMessage>(&self) -> Option<actix::Recipient<RpcStreamCall<T>>> {
        if let Some(r) = self
            .inner
            .recipient()
            .downcast_ref::<actix::Recipient<RpcStreamCall<T>>>()
        {
            Some(r.clone())
        } else {
            None
        }
    }

    fn send(&self, msg: RpcRawCall) -> impl Future01<Item = Vec<u8>, Error = Error> {
        self.inner.send(msg)
    }

    fn send_streaming(
        &self,
        msg: RpcRawCall,
    ) -> impl Stream01<Item = ResponseChunk, Error = Error> {
        self.inner.call_stream(msg)
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

    pub fn bind_stream<T: RpcStreamMessage>(
        &mut self,
        addr: &str,
        endpoint: impl RpcStreamHandler<T> + 'static,
    ) -> Handle {
        let slot = Slot::from_stream_handler(endpoint);
        let addr = format!("{}/{}", addr, T::ID);
        let _ = self.handlers.insert(addr.clone(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr.into()));
        Handle { _inner: () }
    }

    pub fn bind_stream_actor<T: RpcStreamMessage>(
        &mut self,
        addr: &str,
        endpoint: Recipient<RpcStreamCall<T>>,
    ) {
        let slot = Slot::from_stream_actor(endpoint);
        let addr = format!("{}/{}", addr, T::ID);
        let _ = self.handlers.insert(addr.clone(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr));
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
    ) -> impl futures_01::future::Future<Item = Result<T::Item, T::Error>, Error = Error> {
        let caller = "local";
        let addr = format!("{}/{}", addr, T::ID);
        if let Some(slot) = self.handlers.get_mut(&addr) {
            futures_01::future::Either::A(if let Some(h) = slot.recipient() {
                futures_01::future::Either::A(h.send(RpcEnvelope::local(msg)).map_err(Error::from))
            } else {
                let body = rmp_serde::to_vec(&msg).unwrap();
                futures_01::future::Either::B(
                    slot.send(RpcRawCall {
                        caller: caller.into(),
                        addr,
                        body,
                    })
                    .and_then(|b| Ok(rmp_serde::from_read_ref(&b)?)),
                )
            })
        } else {
            let body = rmp_serde::to_vec(&msg).unwrap();
            futures_01::future::Either::B(
                RemoteRouter::from_registry()
                    .send(RpcRawCall {
                        caller: caller.into(),
                        addr,
                        body,
                    })
                    .flatten()
                    .and_then(|b| Ok(rmp_serde::from_read_ref(&b)?)),
            )
        }
    }

    pub fn streaming_forward<T: RpcStreamMessage>(
        &mut self,
        addr: &str,
        msg: T,
    ) -> impl futures_01::stream::Stream<Item = Result<T::Item, T::Error>, Error = Error> {
        let caller = "local";
        let addr = format!("{}/{}", addr, T::ID);
        if let Some(slot) = self.handlers.get_mut(&addr) {
            futures_01::future::Either::A(if let Some(h) = slot.stream_recipient() {
                let (tx, rx) = futures_01::sync::mpsc::channel(16);
                let call = RpcStreamCall {
                    caller: caller.to_string(),
                    addr: addr.to_string(),
                    body: msg,
                    reply: tx,
                };
                Arbiter::spawn(
                    h.send(call)
                        .map_err(|e| log::error!("streaming forward error: {}", e))
                        .map(|_| ()),
                );
                rx.then(|v| match v {
                    Ok(v) => Ok(v),
                    Err(()) => Err(Error::Closed),
                })
            } else {
                /*let body = rmp_serde::to_vec(&msg).unwrap();
                futures_01::future::Either::B(
                    slot.send(RpcRawCall {
                        caller: caller.into(),
                        addr,
                        body,
                    })
                        .and_then(|b| Ok(rmp_serde::from_read_ref(&b)?)),
                )*/
                unimplemented!()
            })
        } else {
            futures_01::future::Either::B(futures_01::stream::once(Err(Error::NoEndpoint)))
        }
    }

    pub fn forward_bytes(
        &mut self,
        addr: &str,
        from: &str,
        msg: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, Error>> {
        if let Some(slot) = self.handlers.get_mut(addr) {
            future::Either::Left(
                slot.send(RpcRawCall {
                    caller: from.into(),
                    addr: addr.into(),
                    body: msg,
                })
                .compat(),
            )
        } else {
            future::Either::Right(
                RemoteRouter::from_registry()
                    .send(RpcRawCall {
                        caller: from.into(),
                        addr: addr.into(),
                        body: msg,
                    })
                    .flatten()
                    .compat(),
            )
        }
    }

    pub fn forward_bytes_local(
        &mut self,
        addr: &str,
        from: &str,
        msg: &[u8],
    ) -> impl Stream<Item = Result<ResponseChunk, Error>> {
        if let Some(slot) = self.handlers.get_mut(addr) {
            future::Either::Left(
                futures::compat::Stream01CompatExt::compat(
                slot.send_streaming(RpcRawCall {
                    caller: from.into(),
                    addr: addr.into(),
                    body: msg.into(),
                }))
            )
        } else {
            log::warn!("no endpoint: {}", addr);
            future::Either::Right(futures::stream::once(async {
                Err(Error::NoEndpoint)
            }))
        }
    }
}

lazy_static::lazy_static! {
static ref ROUTER: Arc<Mutex<Router>> = Arc::new(Mutex::new(Router::new()));
}

pub fn router() -> Arc<Mutex<Router>> {
    ROUTER.clone()
}
