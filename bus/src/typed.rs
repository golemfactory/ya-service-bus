use crate::error::Error;
use crate::local_router::{router, Router};
use crate::{Handle, RpcEndpoint, RpcHandler, RpcMessage, RpcStreamHandler, RpcStreamMessage};
use futures::prelude::*;
use futures::FutureExt;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Binds RpcHandler to given service address.
///
/// ## Example
///
/// ```rust
/// use ya_service_bus::{typed as bus, RpcMessage};
/// use serde::{Serialize, Deserialize};
/// use actix::System;
///
/// #[derive(Serialize, Deserialize)]
/// struct Echo(String);
///
/// impl RpcMessage for Echo {
///     const ID :&'static str = "echo";
///     type Item = String;
///     type Error=();
/// }
///
/// fn main() {
///      let sys = System::new("test");
///      let _ = bus::bind("/local/echo", |e:Echo| {
///          async {
///             Ok(e.0)
///          }
///      });
///  }
///
#[inline]
pub fn bind<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + Unpin + 'static) -> Handle {
    router().lock().unwrap().bind(addr, endpoint)
}

#[doc(hidden)]
#[deprecated(note = "use bind instead")]
pub fn bind_private<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + 'static) -> Handle {
    let addr = format!("/private{}", addr);
    router().lock().unwrap().bind(&addr, endpoint)
}
#[doc(hidden)]
#[deprecated(note = "use bind instead")]
pub fn bind_public<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + 'static) -> Handle {
    let addr = format!("/public{}", addr);
    router().lock().unwrap().bind(&addr, endpoint)
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
pub struct Endpoint {
    router: Arc<Mutex<Router>>,
    addr: String,
}

impl Endpoint {
    pub fn call<T: RpcMessage + Unpin>(
        &self,
        msg: T,
    ) -> impl Future<Output = Result<Result<T::Item, T::Error>, Error>> + Unpin {
        self.router.lock().unwrap().forward(&self.addr, None, msg)
    }

    pub fn call_streaming<T: RpcStreamMessage>(
        &self,
        msg: T,
    ) -> impl Stream<Item = Result<Result<T::Item, T::Error>, Error>> + Unpin {
        self.router
            .lock()
            .unwrap()
            .streaming_forward(&self.addr, msg)
    }
}

impl<T: RpcMessage + Unpin> RpcEndpoint<T> for Endpoint
where
    T: Send,
{
    type Result = Pin<Box<dyn Future<Output = Result<Result<T::Item, T::Error>, Error>>>>;

    fn send(&self, msg: T) -> Self::Result {
        Endpoint::call(self, msg).boxed_local()
    }
}

pub fn service(addr: impl Into<String>) -> Endpoint {
    Endpoint {
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

impl<
        T: RpcStreamMessage,
        Output: Stream<Item = Result<T::Item, T::Error>> + Unpin + 'static,
        F: FnMut(T) -> Output + 'static,
    > RpcStreamHandler<T> for F
{
    type Result = Output;

    fn handle(&mut self, caller: &str, msg: T) -> Self::Result {
        self(msg)
    }
}
