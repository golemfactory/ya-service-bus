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
) -> impl Future<Output = Result<Vec<u8>, Error>> + Unpin {
    router()
        .lock()
        .unwrap()
        .forward_bytes(addr, caller, bytes.into())
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

pub trait RawHandler {
    type Result: Future<Output = Result<Vec<u8>, Error>>;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8]) -> Self::Result;
}

impl<
        Output: Future<Output = Result<Vec<u8>, Error>>,
        F: FnMut(&str, &str, &[u8]) -> Output + 'static,
    > RawHandler for F
{
    type Result = Output;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8]) -> Self::Result {
        self(caller, addr, msg)
    }
}

pub trait RawStreamHandler {
    type Result: Stream<Item = Result<ResponseChunk, Error>>;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8]) -> Self::Result;
}

impl<
        Output: Stream<Item = Result<ResponseChunk, Error>>,
        F: FnMut(&str, &str, &[u8]) -> Output + 'static,
    > RawStreamHandler for F
{
    type Result = Output;

    fn handle(&mut self, caller: &str, addr: &str, msg: &[u8]) -> Self::Result {
        self(caller, addr, msg)
    }
}

impl RawStreamHandler for () {
    type Result = Pin<Box<dyn Stream<Item = Result<ResponseChunk, Error>>>>;

    fn handle(&mut self, _: &str, addr: &str, _: &[u8]) -> Self::Result {
        let addr = addr.to_string();
        futures::stream::once(async { Err(Error::NoEndpoint(addr)) }).boxed_local()
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
                    .handle(&msg.caller, &msg.addr, msg.body.as_ref())
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
            let stream = self
                .stream_handler
                .handle(&msg.caller, &msg.addr, msg.body.as_ref());
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
