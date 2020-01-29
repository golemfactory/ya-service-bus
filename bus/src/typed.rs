use crate::error::Error;
use crate::local_router::{router, Router};
use crate::{Handle, RpcEndpoint, RpcHandler, RpcMessage};
use futures::prelude::*;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub fn bind_private<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + 'static) -> Handle {
    let addr = format!("/private{}", addr);
    router().lock().unwrap().bind(&addr, endpoint)
}

pub fn bind_public<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + 'static) -> Handle {
    let addr = format!("/public{}", addr);
    router().lock().unwrap().bind(&addr, endpoint)
}

#[inline]
pub fn bind<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + 'static) -> Handle {
    router().lock().unwrap().bind(addr, endpoint)
}

#[inline]
pub fn bind_with_caller<T: RpcMessage, Output, F>(addr: &str, f: F) -> Handle
where
    Output: Future<Output = Result<T::Item, T::Error>> + 'static,
    F: FnMut(String, T) -> Output + 'static,
{
    router().lock().unwrap().bind(addr, WithCaller(f))
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

impl<
        T: RpcMessage,
        Output: Future<Output = Result<T::Item, T::Error>> + 'static,
        F: FnMut(T) -> Output + 'static,
    > RpcHandler<T> for F
{
    type Result = Output;

    fn handle(&mut self, _caller: String, msg: T) -> Self::Result {
        self(msg)
    }
}

struct WithCaller<F>(F);

impl<
        T: RpcMessage,
        Output: Future<Output = Result<T::Item, T::Error>> + 'static,
        F: FnMut(String, T) -> Output + 'static,
    > RpcHandler<T> for WithCaller<F>
{
    type Result = Output;

    fn handle(&mut self, caller: String, msg: T) -> Self::Result {
        (self.0)(caller, msg)
    }
}
