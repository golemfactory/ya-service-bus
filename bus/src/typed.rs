use crate::error::Error;
use crate::local_router::{router, Router};
use crate::{Handle, RpcEndpoint, RpcEnvelope, RpcHandler, RpcMessage};
use actix::Message;
use failure::_core::marker::PhantomData;
use futures::compat::{Compat01As03, Future01CompatExt};
use futures::{Future, FutureExt};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub fn bind<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T>) -> Handle {
    unimplemented!()
}

#[derive(Clone)]
struct Forward {
    router: Arc<Mutex<Router>>,
    addr: String,
}

impl<T: RpcMessage> RpcEndpoint<T> for Forward {
    type Result = Pin<Box<dyn Future<Output = Result<Result<T::Item, T::Error>, Error>>>>;

    fn send(&self, msg: T) -> Self::Result {
        self.router
            .lock()
            .unwrap()
            .forward(&self.addr, msg)
            .compat()
            .boxed()
    }
}

pub fn service<T: RpcMessage>(addr: &str) -> impl RpcEndpoint<T> {
    Forward {
        router: router(),
        addr: addr.to_string(),
    }
}
