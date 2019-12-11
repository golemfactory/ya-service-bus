use super::Handle;
use crate::error::Error;
use crate::local_router::{router, Router};
use actix::Actor;
use futures::Future;

pub fn send(addr: &str, from: &str, bytes: &[u8]) -> impl Future<Output = Result<Vec<u8>, Error>> {
    router().lock().unwrap().forward_bytes(addr, from, bytes)
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

mod raw_actor {
    use super::{Error, RawHandler};
    use crate::{RpcEnvelope, RpcRawCall};
    use actix::prelude::*;
    use futures::{FutureExt, TryFutureExt};

    struct RawHandlerActor<T> {
        inner: T,
    }

    impl<T: RawHandler + 'static> Actor for RawHandlerActor<T> {
        type Context = Context<Self>;
    }

    impl<T: RawHandler + 'static> Handler<RpcRawCall> for RawHandlerActor<T> {
        type Result = ActorResponse<Self, Vec<u8>, Error>;

        fn handle(&mut self, msg: RpcRawCall, ctx: &mut Self::Context) -> Self::Result {
            ActorResponse::r#async(
                self.inner
                    .handle(&msg.caller, &msg.addr, msg.body.as_ref())
                    .boxed_local()
                    .compat()
                    .into_actor(self),
            )
        }
    }

    pub fn recipient(h: impl RawHandler + 'static) -> Recipient<RpcRawCall> {
        RawHandlerActor { inner: h }.start().recipient()
    }
}

pub fn subscribe(addr: &str, h: impl RawHandler + 'static) -> Handle {
    router()
        .lock()
        .unwrap()
        .bind_raw(addr, raw_actor::recipient(h))
}
