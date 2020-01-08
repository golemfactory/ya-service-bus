use crate::error::Error;
use crate::local_router::{router, Router};
use crate::{Handle, RpcEndpoint, RpcHandler, RpcMessage};
use futures::prelude::*;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub fn bind<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + 'static) -> Handle {
    router().lock().unwrap().bind(addr, endpoint)
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

pub fn service<T: RpcMessage + Unpin>(addr: &str) -> impl RpcEndpoint<T> {
    Forward {
        router: router(),
        addr: addr.to_string(),
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
