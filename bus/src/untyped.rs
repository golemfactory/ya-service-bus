use super::Handle;
use crate::error::Error;
use crate::local_router::router;

use futures::Future;

pub fn send(
    addr: &str,
    from: &str,
    bytes: &[u8],
) -> impl Future<Output = Result<Vec<u8>, Error>> + Unpin {
    router()
        .lock()
        .unwrap()
        .forward_bytes(addr, from, bytes.into())
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
    use crate::RpcRawCall;
    use actix::prelude::*;
    use futures::{FutureExt};

    struct RawHandlerActor<T> {
        inner: T,
    }

    impl<T: RawHandler + Unpin + 'static> Actor for RawHandlerActor<T> {
        type Context = Context<Self>;
    }

    impl<T: RawHandler + Unpin + 'static> Handler<RpcRawCall> for RawHandlerActor<T> {
        type Result = ActorResponse<Self, Vec<u8>, Error>;

        fn handle(&mut self, msg: RpcRawCall, _ctx: &mut Self::Context) -> Self::Result {
            ActorResponse::r#async(
                self.inner
                    .handle(&msg.caller, &msg.addr, msg.body.as_ref())
                    .boxed_local()
                    .into_actor(self),
            )
        }
    }

    pub fn recipient(h: impl RawHandler + Unpin + 'static) -> Recipient<RpcRawCall> {
        RawHandlerActor { inner: h }.start().recipient()
    }
}

pub fn subscribe(addr: &str, h: impl RawHandler + Unpin + 'static) -> Handle {
    router()
        .lock()
        .unwrap()
        .bind_raw(addr, raw_actor::recipient(h))
}
