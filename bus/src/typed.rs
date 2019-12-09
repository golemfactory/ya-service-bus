use crate::{RpcMessage, RpcHandler, Handle, RpcEndpoint, RpcEnvelope};
use crate::local_router::{Router, router};
use std::sync::{Mutex, Arc};
use failure::_core::marker::PhantomData;
use crate::error::Error;
use futures::compat::{Future01CompatExt, Compat01As03};
use futures::{FutureExt, Future};
use std::pin::Pin;
use actix::Message;

pub fn bind<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T>) -> Handle {
    unimplemented!()
}

#[derive(Clone)]
struct Forward {
    router : Arc<Mutex<Router>>,
    addr : String,
}

impl<T : RpcMessage> RpcEndpoint<T> for Forward {
    type Result = Pin<Box<dyn Future<Output = Result<Result<T::Item, T::Error>, Error>>>>;

    fn send(&self, msg: T) -> Self::Result {
        self.router.lock().unwrap().forward(&self.addr, msg).compat().boxed()
    }
}

pub fn service<T: RpcMessage>(addr: &str) -> impl RpcEndpoint<T> {
    Forward {
        router: router(),
        addr: addr.to_string()
    }
}
