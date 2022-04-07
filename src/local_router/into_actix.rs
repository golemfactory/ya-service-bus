use actix::prelude::*;
use futures::SinkExt;
use std::marker::PhantomData;

use crate::*;

pub struct RpcHandlerWrapper<T, H>(pub(super) H, PhantomData<T>);

impl<T: 'static, H: 'static> Actor for RpcHandlerWrapper<T, H> {
    type Context = Context<Self>;
}

impl<T: 'static, H: 'static> Unpin for RpcHandlerWrapper<T, H> {}

impl<T, H> RpcHandlerWrapper<T, H> {
    pub fn new(h: H) -> Self {
        RpcHandlerWrapper(h, PhantomData)
    }
}

impl<T: RpcMessage, H: RpcHandler<T> + 'static> Handler<RpcEnvelope<T>>
    for RpcHandlerWrapper<T, H>
{
    type Result = ActorResponse<Self, Result<T::Item, T::Error>>;

    fn handle(&mut self, msg: RpcEnvelope<T>, _ctx: &mut Self::Context) -> Self::Result {
        ActorResponse::r#async(self.0.handle(msg.caller, msg.body).into_actor(self))
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

impl<T: 'static, H: 'static> Unpin for RpcStreamHandlerWrapper<T, H> {}

impl<T: RpcStreamMessage, H: RpcStreamHandler<T> + 'static> Handler<RpcStreamCall<T>>
    for RpcStreamHandlerWrapper<T, H>
{
    type Result = ActorResponse<Self, Result<(), Error>>;

    fn handle(&mut self, msg: RpcStreamCall<T>, _ctx: &mut Self::Context) -> Self::Result {
        use futures::stream::StreamExt;

        let mut reply = msg.reply.sink_map_err(|e| Error::GsbFailure(e.to_string()));
        let mut result = self.0.handle(&msg.caller, msg.body).map(Ok);
        let send_all = async move { reply.send_all(&mut result).await };

        ActorResponse::r#async(send_all.into_actor(self))
    }
}
