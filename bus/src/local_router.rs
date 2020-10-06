use actix::{prelude::*, Actor, SystemService};
use futures::{prelude::*, FutureExt, StreamExt};
use std::any::Any;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use ya_sb_util::futures::IntoFlatten;
use ya_sb_util::PrefixLookupBag;

use crate::{
    remote_router::{RemoteRouter, UpdateService},
    Error, Handle, ResponseChunk, RpcEnvelope, RpcHandler, RpcMessage, RpcRawCall,
    RpcRawStreamCall, RpcStreamCall, RpcStreamHandler, RpcStreamMessage,
};

mod into_actix;

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
        let body: T =
            match crate::serialization::from_read(msg.body.as_slice()).map_err(Error::from) {
                Ok(v) => v,
                Err(e) => return future::err(e).boxed_local(),
            };
        Box::pin(
            Recipient::send(self, RpcEnvelope::with_caller(&msg.caller, body))
                .map_err(|e| Error::from_addr("unknown recipient".into(), e))
                .and_then(|r| async move { crate::serialization::to_vec(&r).map_err(Error::from) }),
        )
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>> {
        let body: T =
            match crate::serialization::from_read(msg.body.as_slice()).map_err(Error::from) {
                Ok(v) => v,
                Err(e) => return Box::pin(stream::once(async { Err::<ResponseChunk, Error>(e) })),
            };

        Box::pin(
            Recipient::send(self, RpcEnvelope::with_caller(&msg.caller, body))
                .map_err(|e| Error::from_addr("unknown stream recipient".into(), e))
                .and_then(|r| future::ready(crate::serialization::to_vec(&r).map_err(Error::from)))
                .map_ok(|v| ResponseChunk::Full(v))
                .into_stream(),
        )
    }

    fn recipient(&self) -> &dyn Any {
        self
    }
}

impl<T: RpcStreamMessage> RawEndpoint for Recipient<RpcStreamCall<T>> {
    fn send(&self, _msg: RpcRawCall) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>> {
        Box::pin(future::err(Error::GsbBadRequest(
            "non-streaming-request on streaming endpoint".into(),
        )))
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>> {
        let body: T = crate::serialization::from_read(msg.body.as_slice()).unwrap();
        let (tx, rx) = futures::channel::mpsc::channel(16);
        let (txe, rxe) = futures::channel::oneshot::channel();

        let addr = msg.addr.clone();
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
                    let _ = txe.send(Err(Error::from_addr(addr, e)));
                }
                Ok(Err(e)) => {
                    let _ = txe.send(Err(e));
                }
                Ok(Ok(())) => (),
            };
        });

        let recv_stream = rx
            .then(|r| {
                future::ready(
                    crate::serialization::to_vec(&r)
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
        let addr = msg.addr.clone();
        Box::pin(
            Recipient::<RpcRawCall>::send(self, msg)
                .map_err(|e| Error::from_addr(addr, e))
                .then(|v| async { v? }),
        )
    }

    fn call_stream(
        &self,
        msg: RpcRawCall,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>> {
        let addr = msg.addr.clone();
        Box::pin(
            Recipient::<RpcRawCall>::send(self, msg)
                .map_err(|e| Error::from_addr(addr, e))
                .flatten_fut()
                .and_then(|v| future::ok(ResponseChunk::Full(v)))
                .into_stream(),
        )
    }

    fn recipient(&self) -> &dyn Any {
        self
    }
}

impl RawEndpoint for Recipient<RpcRawStreamCall> {
    fn send(&self, msg: RpcRawCall) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Error>>>> {
        let (tx, rx) = futures::channel::mpsc::channel(1);
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
            .then(|_v| future::ready(())),
        );
        async move {
            futures::pin_mut!(rx);
            match StreamExt::next(&mut rx).await {
                Some(Ok(ResponseChunk::Full(v))) => Ok(v),
                Some(Ok(ResponseChunk::Part(_))) => {
                    Err(Error::GsbBadRequest("partial response".into()))
                }
                Some(Err(e)) => Err(e),
                None => Err(Error::GsbBadRequest("unexpected EOS".into())),
            }
        }
        .boxed_local()
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
            .then(|_| future::ready(())),
        );
        Box::pin(rx)
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

    fn send(&self, msg: RpcRawCall) -> impl Future<Output = Result<Vec<u8>, Error>> + Unpin {
        self.inner.send(msg)
    }

    fn send_streaming(&self, msg: RpcRawCall) -> impl Stream<Item = Result<ResponseChunk, Error>> {
        self.inner.call_stream(msg)
    }

    fn streaming_forward<T: RpcStreamMessage>(
        &self,
        caller: String,
        addr: String,
        body: T,
    ) -> impl Stream<Item = Result<Result<T::Item, T::Error>, Error>> {
        if let Some(h) = self.stream_recipient() {
            let (reply, rx) = futures::channel::mpsc::channel(16);
            let call = RpcStreamCall {
                caller,
                addr,
                body,
                reply,
            };

            Arbiter::spawn(async move {
                h.send(call)
                    .await
                    .unwrap_or_else(|e| Ok(log::error!("streaming forward error: {}", e)))
                    .unwrap_or_else(|e| log::error!("streaming forward error: {}", e));
            });
            rx.map(|v| Ok(v)).left_stream()
        } else {
            (move || {
                let body = match crate::serialization::to_vec(&body) {
                    Ok(body) => body,
                    Err(e) => return stream::once(future::err(Error::from(e))).right_stream(),
                };
                self.send_streaming(RpcRawCall { caller, addr, body })
                    .map(|chunk_result| {
                        (move || -> Result<Result<T::Item, T::Error>, Error> {
                            let chunk = match chunk_result {
                                Ok(ResponseChunk::Part(chunk)) => chunk,
                                Ok(ResponseChunk::Full(chunk)) => chunk,
                                Err(e) => return Err(e),
                            };
                            Ok(crate::serialization::from_read(Cursor::new(chunk))?)
                        })()
                    })
                    .left_stream()
            })()
            .right_stream()
        }
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
        log::debug!("binding {}", addr);
        let _ = self.handlers.insert(addr.clone(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr));
        Handle { _inner: () }
    }

    pub fn unbind(&mut self, addr: &str) -> impl Future<Output = Result<bool, Error>> + Unpin {
        let pattern = match addr.ends_with('/') {
            true => addr.to_string(),
            false => format!("{}/", addr),
        };
        let addrs = self
            .handlers
            .keys()
            .filter(|a| a.starts_with(&pattern))
            .cloned()
            .collect::<Vec<String>>();

        addrs.iter().for_each(|addr| {
            log::debug!("unbinding {}", addr);
            self.handlers.remove(&addr);
        });

        Box::pin(async move {
            let router = RemoteRouter::from_registry();
            let success = !addrs.is_empty();
            for addr in addrs {
                router
                    .send(UpdateService::Remove(addr.clone()))
                    .await
                    .map_err(|e| Error::from_addr(addr, e))?;
            }
            Ok(success)
        })
    }

    pub fn bind_stream<T: RpcStreamMessage>(
        &mut self,
        addr: &str,
        endpoint: impl RpcStreamHandler<T> + Unpin + 'static,
    ) -> Handle {
        let slot = Slot::from_stream_handler(endpoint);
        let addr = format!("{}/{}", addr, T::ID);
        log::debug!("binding stream {}", addr);
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
        log::debug!("binding stream actor {}", addr);
        let _ = self.handlers.insert(addr.clone(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr));
    }

    pub fn bind_actor<T: RpcMessage>(&mut self, addr: &str, endpoint: Recipient<RpcEnvelope<T>>) {
        let slot = Slot::from_actor(endpoint);
        let addr = format!("{}/{}", addr, T::ID);
        log::debug!("binding actor {}", addr);
        let _ = self.handlers.insert(addr.clone(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr));
    }

    pub fn bind_raw(&mut self, addr: &str, endpoint: Recipient<RpcRawCall>) -> Handle {
        let slot = Slot::from_raw(endpoint);
        log::debug!("binding raw {}", addr);
        let _ = self.handlers.insert(addr.to_string(), slot);
        RemoteRouter::from_registry().do_send(UpdateService::Add(addr.into()));
        Handle { _inner: () }
    }

    pub fn forward<T: RpcMessage + Unpin>(
        &mut self,
        addr: &str,
        msg: RpcEnvelope<T>,
    ) -> impl Future<Output = Result<Result<T::Item, T::Error>, Error>> + Unpin {
        let addr = format!("{}/{}", addr, T::ID);
        if let Some(slot) = self.handlers.get_mut(&addr) {
            (if let Some(h) = slot.recipient() {
                h.send(msg)
                    .map_err(|e| Error::from_addr(addr, e))
                    .left_future()
            } else {
                slot.send(RpcRawCall::from_envelope_addr(msg, addr))
                    .then(|b| {
                        future::ready(match b {
                            Ok(b) => crate::serialization::from_read(std::io::Cursor::new(&b))
                                .map_err(From::from),
                            Err(e) => Err(e),
                        })
                    })
                    .right_future()
            })
            .left_future()
        } else {
            RemoteRouter::from_registry()
                .send(RpcRawCall::from_envelope_addr(msg, addr.clone()))
                .then(|v| {
                    future::ready(match v {
                        Ok(v) => v,
                        Err(e) => Err(Error::from_addr(addr, e)),
                    })
                })
                .then(|b| {
                    future::ready(match b {
                        Ok(b) => crate::serialization::from_read(std::io::Cursor::new(&b))
                            .map_err(From::from),
                        Err(e) => Err(e),
                    })
                })
                .right_future()
        }
    }

    pub fn streaming_forward<T: RpcStreamMessage>(
        &mut self,
        addr: &str,
        // TODO: add `from: &str` as in `forward_bytes` below
        msg: T,
    ) -> impl Stream<Item = Result<Result<T::Item, T::Error>, Error>> {
        let caller = "local".to_string();
        let addr = format!("{}/{}", addr, T::ID);
        if let Some(slot) = self.handlers.get_mut(&addr) {
            slot.streaming_forward(caller, addr, msg).left_stream()
        } else {
            //use futures::StreamExt;
            log::debug!("call remote");
            let body = crate::serialization::to_vec(&msg).unwrap();
            let (reply, tx) = futures::channel::mpsc::channel(16);
            let call = RpcRawStreamCall {
                caller,
                addr,
                body,
                reply,
            };
            let _ = Arbiter::spawn(async move {
                let v = RemoteRouter::from_registry().send(call).await;
                log::debug!("call result={:?}", v);
            });

            tx.filter(|s| future::ready(s.as_ref().map(|s| !s.is_eos()).unwrap_or(true)))
                .map(|b| {
                    let body = b?.into_bytes();
                    Ok(crate::serialization::from_read(std::io::Cursor::new(
                        &body,
                    ))?)
                })
                .right_stream()
        }
    }

    pub fn forward_bytes(
        &mut self,
        addr: &str,
        caller: &str,
        msg: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, Error>> + Unpin {
        let addr = addr.to_string();
        if let Some(slot) = self.handlers.get_mut(&addr) {
            slot.send(RpcRawCall {
                caller: caller.into(),
                addr: addr.clone(),
                body: msg,
            })
            .left_future()
        } else {
            RemoteRouter::from_registry()
                .send(RpcRawCall {
                    caller: caller.into(),
                    addr: addr.clone(),
                    body: msg,
                })
                .then(|v| match v {
                    Ok(r) => future::ready(r),
                    Err(e) => future::err(Error::from_addr(addr, e)),
                })
                .right_future()
        }
    }

    pub fn forward_bytes_local(
        &mut self,
        addr: &str,
        caller: &str,
        msg: &[u8],
    ) -> impl Stream<Item = Result<ResponseChunk, Error>> {
        let addr = addr.to_string();
        if let Some(slot) = self.handlers.get_mut(&addr) {
            slot.send_streaming(RpcRawCall {
                caller: caller.into(),
                addr,
                body: msg.into(),
            })
            .left_stream()
        } else {
            log::warn!("no endpoint: {}", addr);
            futures::stream::once(async { Err(Error::NoEndpoint(addr)) }).right_stream()
        }
    }
}

lazy_static::lazy_static! {
static ref ROUTER: Arc<Mutex<Router>> = Arc::new(Mutex::new(Router::new()));
}

pub fn router() -> Arc<Mutex<Router>> {
    (*ROUTER).clone()
}
