use super::Handle;
use crate::error::Error;
use crate::local_router::router;
use crate::ResponseChunk;
use futures::{Future, Stream, StreamExt};
use std::pin::Pin;

pub fn send(
    addr: &str,
    caller: &str,
    bytes: &[u8],
) -> impl Future<Output = Result<Vec<u8>, Error>> {
    forward_bytes(addr, caller, bytes, false)
}

pub fn push(
    addr: &str,
    caller: &str,
    bytes: &[u8],
) -> impl Future<Output = Result<Vec<u8>, Error>> {
    forward_bytes(addr, caller, bytes, true)
}

pub fn call_stream(
    addr: &str,
    caller: &str,
    bytes: &[u8],
) -> Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>> {
    router()
        .lock()
        .unwrap()
        .streaming_forward_bytes(addr, caller, bytes.into())
        .boxed_local()
}

fn forward_bytes(
    addr: &str,
    caller: &str,
    bytes: &[u8],
    no_reply: bool,
) -> impl Future<Output = Result<Vec<u8>, Error>> {
    router()
        .lock()
        .unwrap()
        .forward_bytes(addr, caller, bytes.into(), no_reply)
}

pub trait RawHandler {
    type Result: Future<Output = Result<Vec<u8>, Error>>;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8], no_reply: bool) -> Self::Result;
}

impl<
        Output: Future<Output = Result<Vec<u8>, Error>>,
        F: FnMut(&str, &str, &[u8]) -> Output + 'static,
    > RawHandler for F
{
    type Result = Output;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8], _no_reply: bool) -> Self::Result {
        self(caller, addr, msg)
    }
}

pub trait RawStreamHandler {
    type Result: Stream<Item = Result<ResponseChunk, Error>>;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8], _no_reply: bool) -> Self::Result;
}

impl<
        Output: Stream<Item = Result<ResponseChunk, Error>>,
        F: FnMut(&str, &str, &[u8]) -> Output + 'static,
    > RawStreamHandler for F
{
    type Result = Output;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8], _no_reply: bool) -> Self::Result {
        self(caller, addr, msg)
    }
}

impl RawStreamHandler for () {
    type Result = Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>>;

    fn handle(&mut self, _: &str, addr: &str, _: &[u8], _: bool) -> Self::Result {
        let addr = addr.to_string();
        futures::stream::once(async { Err(Error::NoEndpoint(addr)) }).boxed_local()
    }
}

pub struct Fn4Handler<R> {
    #[allow(clippy::type_complexity)]
    f: Box<dyn FnMut(&str, &str, &[u8], bool) -> R>,
}

impl<Fut> RawHandler for Fn4Handler<Fut>
where
    Fut: Future<Output = Result<Vec<u8>, Error>>,
{
    type Result = Fut;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8], no_reply: bool) -> Self::Result {
        (*self.f)(caller, addr, msg, no_reply)
    }
}

impl<S> RawStreamHandler for Fn4Handler<S>
where
    S: Stream<Item = Result<ResponseChunk, Error>>,
{
    type Result = S;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8], no_reply: bool) -> Self::Result {
        (*self.f)(caller, addr, msg, no_reply)
    }
}

pub trait Fn4HandlerExt<Fut>
where
    Fut: Future<Output = Result<Vec<u8>, Error>>,
{
    fn into_handler(self) -> Fn4Handler<Fut>;
}

impl<F, Fut> Fn4HandlerExt<Fut> for F
where
    Fut: Future<Output = Result<Vec<u8>, Error>>,
    F: FnMut(&str, &str, &[u8], bool) -> Fut + 'static,
{
    fn into_handler(self) -> Fn4Handler<Fut> {
        Fn4Handler { f: Box::new(self) }
    }
}

pub trait Fn4StreamHandlerExt<S>
where
    S: Stream<Item = Result<ResponseChunk, Error>>,
{
    fn into_stream_handler(self) -> Fn4Handler<S>;
}

impl<F, S> Fn4StreamHandlerExt<S> for F
where
    S: Stream<Item = Result<ResponseChunk, Error>>,
    F: FnMut(&str, &str, &[u8], bool) -> S + 'static,
{
    fn into_stream_handler(self) -> Fn4Handler<S> {
        Fn4Handler { f: Box::new(self) }
    }
}

mod raw_actor {
    use super::{Error, RawHandler};
    use crate::untyped::RawStreamHandler;
    use crate::{RpcRawCall, RpcRawStreamCall};
    use actix::prelude::*;
    use futures::{FutureExt, SinkExt, StreamExt};

    struct RawHandlerActor<H, S> {
        handler: H,
        stream_handler: S,
    }

    impl<H: Unpin + 'static, S: Unpin + 'static> Actor for RawHandlerActor<H, S> {
        type Context = Context<Self>;
    }

    impl<H: RawHandler + Unpin + 'static, S: Unpin + 'static> Handler<RpcRawCall>
        for RawHandlerActor<H, S>
    {
        type Result = ActorResponse<Self, Result<Vec<u8>, Error>>;

        fn handle(&mut self, msg: RpcRawCall, _ctx: &mut Self::Context) -> Self::Result {
            ActorResponse::r#async(
                self.handler
                    .handle(&msg.caller, &msg.addr, msg.body.as_ref(), msg.no_reply)
                    .boxed_local()
                    .into_actor(self),
            )
        }
    }

    impl<H: Unpin + 'static, S: RawStreamHandler + Unpin + 'static> Handler<RpcRawStreamCall>
        for RawHandlerActor<H, S>
    {
        type Result = Result<(), Error>;

        fn handle(&mut self, msg: RpcRawStreamCall, ctx: &mut Self::Context) -> Self::Result {
            let stream =
                self.stream_handler
                    .handle(&msg.caller, &msg.addr, msg.body.as_ref(), false);
            let sink = msg
                .reply
                .sink_map_err(|e| Error::GsbFailure(e.to_string()))
                .with(|r| futures::future::ready(Ok(Ok(r))));

            ctx.spawn(stream.forward(sink).map(|_| ()).into_actor(self));
            Ok(())
        }
    }

    pub fn recipients(
        h: impl RawHandler + Unpin + 'static,
        s: impl RawStreamHandler + Unpin + 'static,
    ) -> (Recipient<RpcRawCall>, Recipient<RpcRawStreamCall>) {
        let addr = RawHandlerActor {
            handler: h,
            stream_handler: s,
        }
        .start();
        (addr.clone().recipient(), addr.recipient())
    }
}

pub fn subscribe(
    addr: &str,
    rpc: impl RawHandler + Unpin + 'static,
    stream: impl RawStreamHandler + Unpin + 'static,
) -> Handle {
    let (rr, rs) = raw_actor::recipients(rpc, stream);
    router().lock().unwrap().bind_raw_dual(addr, rr, rs)
}
