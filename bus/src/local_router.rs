use crate::{
    error::Error, Handle, ResponseChunk, RpcEnvelope, RpcHandler, RpcMessage, RpcRawCall,
    RpcRawStreamCall, RpcStreamCall, RpcStreamHandler, RpcStreamMessage,
};
use actix::prelude::*;
use futures::prelude::*;
use futures::{future, FutureExt, StreamExt, TryStreamExt, SinkExt};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::pin::Pin;
use ya_sb_util::PrefixLookupBag;

mod into_actix;

use crate::local_router::into_actix::RpcHandlerWrapper;
use crate::remote_router::{RemoteRouter, UpdateService};
use crate::untyped::RawHandler;
use crate::ResponseChunk::Full;
use actix::{Actor, SystemService};
use futures::future::ErrInto;
use std::sync::{Arc, Mutex};
use ya_sb_util::futures::IntoFlatten;

trait RawEndpoint: Any {
    fn send(&self, msg: RpcRawCall) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>>;

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>>;

    fn recipient(&self) -> &dyn Any;
}

// Implementation for non-streaming service
impl<T: RpcMessage> RawEndpoint for Recipient<RpcEnvelope<T>> {
    fn send(&self, msg: RpcRawCall) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>> {
        let body: T = rmp_serde::decode::from_read(msg.body.as_slice()).unwrap();
        Box::pin(
            Recipient::send(self, RpcEnvelope::with_caller(&msg.caller, body))
                .map_err(|e| e.into())
                .and_then(|r| async move { rmp_serde::to_vec(&r).map_err(Error::from) }),
        )
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>> {
        let body: T = rmp_serde::decode::from_read(msg.body.as_slice()).unwrap();

        Box::pin(
            Recipient::send(self, RpcEnvelope::with_caller(&msg.caller, body))
                .map_err(|e| e.into())
                .and_then(|r| future::ready(rmp_serde::to_vec(&r).map_err(Error::from)))
                .map_ok(|v| ResponseChunk::Full(v))
                .into_stream(),
        )
    }

    fn recipient(&self) -> &dyn Any {
        self
    }
}

impl<T: RpcStreamMessage> RawEndpoint for Recipient<RpcStreamCall<T>> {
    fn send(&self, msg: RpcRawCall) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>> {
        Box::pin(future::err(Error::GsbBadRequest(
            "non-streaming-request on streaming endpoint".into(),
        )))
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>> {
        let body: T = rmp_serde::decode::from_read(msg.body.as_slice()).unwrap();
        let (mut tx, rx) = futures::channel::mpsc::channel(16);
        let (mut txe, rxe) = futures::channel::oneshot::channel();

        let call = RpcStreamCall {
            caller: msg.caller,
            addr: msg.addr,
            body,
            reply: tx,
        };
        let me = self.clone();
        Arbiter::spawn(async move {
            match me.send(call).await {
                Err(e) => {
                    txe.send(Err(e.into()));
                }
                Ok(Err(e)) => {
                    txe.send(Err(e));
                }
                Ok(Ok(s)) => (),
            };
        });

        let recv_stream = rx
            .then(|r| {
                future::ready(
                    rmp_serde::to_vec(&r)
                        .map_err(Error::from)
                        .and_then(|r| Ok(ResponseChunk::Part(r))),
                )
            })
            .chain(rxe.into_stream().filter_map(|v| future::ready(v.ok())));

        Box::pin(recv_stream)
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

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>> {
        Box::pin(
            Recipient::<RpcRawCall>::send(self, msg)
                .map_err(Error::from)
                .flatten_fut()
                .and_then(|v| future::ok(ResponseChunk::Full(v)))
                .into_stream(),
        )
    }

    fn recipient(&self) -> &Any {
        self
    }
}

impl RawEndpoint for Recipient<RpcRawStreamCall> {
    fn send(&self, msg: RpcRawCall) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>> {
        let (tx, mut rx) = futures::channel::mpsc::channel(1);
        // TODO: send error to caller
        Arbiter::spawn(
            self.send(RpcRawStreamCall {
                caller: msg.caller,
                addr: msg.addr,
                body: msg.body,
                reply: tx,
            })
            .flatten_fut()
            .map_err(|e| eprintln!("cell error={}", e))
                .then(|v| future::ready(()))
        );
        async move {
            futures::pin_mut!(rx);
            if let Some(ResponseChunk::Full(v)) = StreamExt::next(&mut rx).await {
                Ok(v)
            }
            else {
                Err(Error::GsbBadRequest("partial response".into()))
            }
        }.boxed_local()
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>> {
        let (tx, rx) = futures::channel::mpsc::channel(16);
        // TODO: send error to caller
        Arbiter::spawn(
            self.send(RpcRawStreamCall {
                caller: msg.caller,
                addr: msg.addr,
                body: msg.body,
                reply: tx,
            })
            .flatten_fut()
            .map_err(|e| eprintln!("cell error={}", e))
                .then(|_| future::ready(()))
        );
        Box::pin(rx.map(|v| Ok(v)))
    }

    fn recipient(&self) -> &Any {
        self
    }
}

struct Slot {
    inner: Box<dyn RawEndpoint + Send + 'static>,
}

impl Slot {
    fn from_handler<T: RpcMessage, H: RpcHandler<T> + 'static + Unpin>(handler: H) -> Self {
        Slot {
            inner: Box::new(
                into_actix::RpcHandlerWrapper::new(handler)
                    .start()
                    .recipient(),
            ),
        }
    }

    fn from_stream_handler<T: RpcStreamMessage + Unpin, H: RpcStreamHandler<T> + 'static + Unpin>(
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

    fn send(&self, msg: RpcRawCall) -> impl Future<Output = Result<Vec<u8>, Error>> + Unpin {
        self.inner.send(msg)
    }

    fn send_streaming(&self, msg: RpcRawCall) -> impl Stream<Item = Result<ResponseChunk, Error>> {
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
        endpoint: impl RpcHandler<T> + 'static + Unpin,
    ) -> Handle {
        let slot = Slot::from_handler(endpoint);
        let addr = format!("{}/{}", addr, T::ID);
        let _ = self.handlers.insert(addr.clone(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr.into()));
        Handle { _inner: () }
    }

    pub fn bind_stream<T: RpcStreamMessage + Unpin>(
        &mut self,
        addr: &str,
        endpoint: impl RpcStreamHandler<T> + Unpin + 'static,
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

    pub fn forward<T: RpcMessage + Unpin>(
        &mut self,
        addr: &str,
        msg: T,
    ) -> impl Future<Output = Result<Result<T::Item, T::Error>, Error>> + Unpin {
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
                .then(|b| {
                    future::ready(match b {
                        Ok(b) => rmp_serde::from_read_ref(&b).map_err(From::from),
                        Err(e) => Err(e),
                    })
                })
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
                .then(|v| {
                    future::ready(match v {
                        Ok(v) => v,
                        Err(e) => Err(e.into()),
                    })
                })
                .then(|b| {
                    future::ready(match b {
                        Ok(b) => rmp_serde::from_read_ref(&b).map_err(From::from),
                        Err(e) => Err(e),
                    })
                })
                .right_future()
        }
    }

    pub fn streaming_forward<T: RpcStreamMessage>(
        &mut self,
        addr: &str,
        msg: T,
    ) -> impl Stream<Item = Result<Result<T::Item, T::Error>, Error>> {
        let caller = "local";
        let addr = format!("{}/{}", addr, T::ID);
        if let Some(slot) = self.handlers.get_mut(&addr) {
            (if let Some(h) = slot.stream_recipient() {
                let (tx, rx) = futures::channel::mpsc::channel(16);
                let call = RpcStreamCall {
                    caller: caller.to_string(),
                    addr: addr.to_string(),
                    body: msg,
                    reply: tx,
                };
                Arbiter::spawn(async move {
                    h.send(call)
                        .await
                        .unwrap_or_else(|e| Ok(log::error!("streaming forward error: {}", e)))
                        .unwrap_or_else(|e| log::error!("streaming forward error: {}", e));
                });
                rx.map(|v| Ok(v))
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
            .left_stream()
        } else {
            futures::stream::once(future::ready(Err(Error::NoEndpoint))).right_stream()
        }
    }

    pub fn forward_bytes(
        &mut self,
        addr: &str,
        from: &str,
        msg: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, Error>> + Unpin {
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
                .then(|v| match v {
                    Ok(r) => future::ready(r),
                    Err(e) => future::err(e.into()),
                })
                .right_future()
        }
    }

    pub fn forward_bytes_local(
        &mut self,
        addr: &str,
        from: &str,
        msg: &[u8],
    ) -> impl Stream<Item = Result<ResponseChunk, Error>> {
        if let Some(slot) = self.handlers.get_mut(addr) {
            slot.send_streaming(RpcRawCall {
                caller: from.into(),
                addr: addr.into(),
                body: msg.into(),
            })
            .left_stream()
        } else {
            log::warn!("no endpoint: {}", addr);
            futures::stream::once(async { Err(Error::NoEndpoint) }).right_stream()
        }
    }
}

lazy_static::lazy_static! {
static ref ROUTER: Arc<Mutex<Router>> = Arc::new(Mutex::new(Router::new()));
}

pub fn router() -> Arc<Mutex<Router>> {
    (*ROUTER).clone()
}
