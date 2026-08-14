use crate::{RpcRawCall, RpcStreamCall, RpcStreamMessage};
use actix::prelude::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::{Arc, Mutex};

use crate::local_router::{router, Router};
use crate::{RpcEnvelope, RpcMessage};

use super::error::Error as BusError;
use super::Handle;
use futures::Future;

pub fn bind<M: RpcMessage>(addr: &str, actor: Recipient<RpcEnvelope<M>>) -> Handle
where
    <RpcEnvelope<M> as Message>::Result: Serialize + DeserializeOwned + Sync + Send,
{
    router().lock().unwrap().bind_actor(addr, actor);
    Handle { _inner: {} }
}

pub fn bind_raw(addr: &str, actor: Recipient<RpcRawCall>) -> Handle {
    router().lock().unwrap().bind_raw(addr, actor);
    Handle { _inner: {} }
}

/// Binds a raw endpoint without replacing an existing handler registered at
/// the same exact address.
pub fn try_bind_raw(addr: &str, actor: Recipient<RpcRawCall>) -> Result<Handle, BusError> {
    router().lock().unwrap().try_bind_raw(addr, actor)
}

/// Unbinds only the raw endpoint registered at the exact address.
pub fn unbind_raw(addr: &str) -> bool {
    router().lock().unwrap().unbind_exact(addr)
}

pub fn binds<M: RpcStreamMessage>(addr: &str, actor: Recipient<RpcStreamCall<M>>) -> Handle
where
    Result<M::Item, M::Error>: Serialize + DeserializeOwned + Sync + Send,
{
    router().lock().unwrap().bind_stream_actor(addr, actor);
    Handle { _inner: {} }
}

pub fn service(addr: &str) -> Endpoint {
    Endpoint {
        addr: addr.to_string(),
        router: router(),
    }
}

pub struct Endpoint {
    addr: String,
    router: Arc<Mutex<Router>>,
}

impl Endpoint {
    pub fn send<M: RpcMessage + Serialize + DeserializeOwned + Sync + Send + Unpin>(
        &self,
        msg: M,
    ) -> impl Future<Output = Result<<RpcEnvelope<M> as Message>::Result, BusError>> + 'static {
        let mut b = self.router.lock().unwrap();
        b.forward(self.addr.as_ref(), RpcEnvelope::local(msg))
    }

    pub fn send_as<M: RpcMessage + Serialize + DeserializeOwned + Sync + Send + Unpin>(
        &self,
        caller: impl ToString,
        msg: M,
    ) -> impl Future<Output = Result<<RpcEnvelope<M> as Message>::Result, BusError>> + 'static {
        let mut b = self.router.lock().unwrap();
        b.forward(self.addr.as_ref(), RpcEnvelope::with_caller(caller, msg))
    }

    pub fn push<M: RpcMessage + Serialize + DeserializeOwned + Sync + Send + Unpin>(
        &self,
        msg: M,
    ) -> impl Future<Output = Result<(), BusError>> + 'static {
        let mut b = self.router.lock().unwrap();
        b.push(self.addr.as_ref(), RpcEnvelope::local(msg))
    }

    pub fn push_as<M: RpcMessage + Serialize + DeserializeOwned + Sync + Send + Unpin>(
        &self,
        caller: impl ToString,
        msg: M,
    ) -> impl Future<Output = Result<(), BusError>> + 'static {
        let mut b = self.router.lock().unwrap();
        b.push(self.addr.as_ref(), RpcEnvelope::with_caller(caller, msg))
    }

    pub fn call_stream<M: RpcStreamMessage>(
        &self,
        // TODO: add caller
        msg: M,
    ) -> impl Stream<Item = Result<Result<M::Item, M::Error>, BusError>> {
        self.router
            .lock()
            .unwrap()
            .streaming_forward(&self.addr, msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix::{Actor, Context, Handler};

    struct RawActor;

    impl Actor for RawActor {
        type Context = Context<Self>;
    }

    impl Handler<RpcRawCall> for RawActor {
        type Result = Result<Vec<u8>, BusError>;

        fn handle(&mut self, message: RpcRawCall, _ctx: &mut Self::Context) -> Self::Result {
            Ok(message.body)
        }
    }

    #[actix_rt::test]
    async fn checked_raw_binding_never_replaces_and_exact_unbind_allows_rebind() {
        let service = RawActor.start();
        let address = format!("/local/test/{}", uuid::Uuid::new_v4());

        try_bind_raw(&address, service.clone().recipient()).unwrap();
        let error = match try_bind_raw(&address, service.clone().recipient()) {
            Ok(_) => panic!("duplicate binding unexpectedly replaced the handler"),
            Err(error) => error,
        };
        assert!(matches!(error, BusError::GsbAlreadyRegistered(_)));

        assert!(unbind_raw(&address));
        try_bind_raw(&address, service.recipient()).unwrap();
        assert!(unbind_raw(&address));
        assert!(!unbind_raw(&address));
    }
}
