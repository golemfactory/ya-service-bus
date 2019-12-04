use crate::error::Error;
use crate::{RpcEnvelope, RpcMessage};
use actix::prelude::*;
use failure::_core::marker::PhantomData;

pub struct RpcHandlerWrapper<T, H>(pub(super) H, PhantomData<T>);

impl<T: 'static, H: 'static> Actor for RpcHandlerWrapper<T, H> {
    type Context = Context<Self>;
}

impl<T, H> RpcHandlerWrapper<T, H> {
    pub fn new(h: H) -> Self {
        RpcHandlerWrapper(h, PhantomData)
    }
}

impl<T: RpcMessage, H: 'static> Handler<RpcEnvelope<T>> for RpcHandlerWrapper<T, H> {
    type Result = ActorResponse<Self, T::Item, T::Error>;

    fn handle(&mut self, msg: RpcEnvelope<T>, ctx: &mut Self::Context) -> Self::Result {
        unimplemented!()
    }
}