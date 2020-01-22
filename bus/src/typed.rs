use crate::error::Error;
use crate::local_router::{router, Router};
use crate::{Handle, RpcEndpoint, RpcHandler, RpcMessage};
use futures::prelude::*;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use ya_service_api::constants::{PRIVATE_SERVICE, PUBLIC_SERVICE};

pub fn bind_private<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + 'static) -> Handle {
    router()
        .lock()
        .unwrap()
        .bind(PRIVATE_SERVICE, addr, endpoint)
}

pub fn bind_public<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + 'static) -> Handle {
    router()
        .lock()
        .unwrap()
        .bind(PUBLIC_SERVICE, addr, endpoint)
}

#[derive(Clone)]
struct Forward {
    router: Arc<Mutex<Router>>,
    addr: String,
}

impl<T: RpcMessage> RpcEndpoint<T> for Forward
where
    T: Send + Unpin,
{
    type Result = Pin<Box<dyn Future<Output = Result<Result<T::Item, T::Error>, Error>>>>;

    fn send(&self, msg: T) -> Self::Result {
        self.router
            .lock()
            .unwrap()
            .forward(&self.addr, msg)
            .boxed_local()
    }
}

pub fn service<T: RpcMessage + Unpin>(addr: impl Into<String>) -> impl RpcEndpoint<T> {
    Forward {
        router: router(),
        addr: addr.into(),
    }
}

pub fn private_service<T: RpcMessage + Unpin>(addr: &str) -> impl RpcEndpoint<T> {
    Forward {
        router: router(),
        addr: format!("{}{}", PRIVATE_SERVICE, addr),
    }
}

impl<
        T: RpcMessage,
        Output: Future<Output = Result<T::Item, T::Error>> + 'static,
        F: FnMut(T) -> Output + 'static,
    > RpcHandler<T> for F
{
    type Result = Output;

    fn handle(&mut self, _caller: &str, msg: T) -> Self::Result {
        self(msg)
    }
}
