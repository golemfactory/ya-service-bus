use crate::error::Error;
use crate::{
    RpcEnvelope, RpcHandler, RpcMessage, RpcStreamCall, RpcStreamHandler, RpcStreamMessage,
};
use actix::prelude::*;
use futures::{FutureExt, Sink, TryFutureExt};
use std::marker::PhantomData;

pub struct RpcHandlerWrapper<T, H>(pub(super) H, PhantomData<T>);

impl<T: 'static, H: 'static> Actor for RpcHandlerWrapper<T, H> {
    type Context = Context<Self>;
}

impl<T, H> RpcHandlerWrapper<T, H> {
    pub fn new(h: H) -> Self {
        RpcHandlerWrapper(h, PhantomData)
    }
}

impl<T: RpcMessage, H: RpcHandler<T> + 'static> Handler<RpcEnvelope<T>>
    for RpcHandlerWrapper<T, H>
{
    type Result = ActorResponse<Self, T::Item, T::Error>;

    fn handle(&mut self, msg: RpcEnvelope<T>, ctx: &mut Self::Context) -> Self::Result {
        ActorResponse::r#async(
            self.0
                .handle(msg.caller.as_str(), msg.body)
                .boxed_local()
                .compat()
                .into_actor(self),
        )
    }
}

pub struct RpcStreamHandlerWrapper<T, H>(pub(super) H, PhantomData<T>);

impl<T, H> RpcStreamHandlerWrapper<T, H> {
    pub fn new(h: H) -> Self {
        RpcStreamHandlerWrapper(h, PhantomData)
    }
}

impl<T: 'static, H: 'static> Actor for RpcStreamHandlerWrapper<T, H> {
    type Context = Context<Self>;
}

impl<T: RpcStreamMessage, H: RpcStreamHandler<T> + 'static> Handler<RpcStreamCall<T>>
    for RpcStreamHandlerWrapper<T, H>
{
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: RpcStreamCall<T>, ctx: &mut Self::Context) -> Self::Result {
        use futures::stream::{Stream, TryStream, TryStreamExt};
        use futures_01::prelude::*;

        let mut s = self.0.handle(&msg.caller, msg.body);
        /*
        let pump = msg
            .reply
            .send_all(s.compat())
            .map_err(|e| ())
            .and_then(|(reply, stream)| Ok(()));
        ActorResponse::r#async(pump.into_actor(self))
        */
        unimplemented!()
    }
}
