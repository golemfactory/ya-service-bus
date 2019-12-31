use crate::error::Error;
use crate::local_router::{router, Router};
use crate::{
    Handle, RpcEndpoint, RpcEnvelope, RpcHandler, RpcMessage, RpcStreamHandler, RpcStreamMessage,
};
use actix::Message;
use failure::_core::marker::PhantomData;
use futures::compat::{Compat01As03, Future01CompatExt, Stream01CompatExt};
use futures::{Future, FutureExt, Stream};
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
pub fn bind<T: RpcMessage>(addr: &str, endpoint: impl RpcHandler<T> + 'static) -> Handle {
    router().lock().unwrap().bind(addr, endpoint)
}

pub fn bind_streaming<T: RpcStreamMessage>(
    addr: &str,
    endpoint: impl RpcStreamHandler<T> + 'static,
) -> Handle {
    router().lock().unwrap().bind_stream(addr, endpoint)
}

#[derive(Clone)]
pub struct Endpoint {
    router: Arc<Mutex<Router>>,
    addr: String,
}

impl Endpoint {
    pub fn call<T: RpcMessage>(
        &self,
        msg: T,
    ) -> impl Future<Output = Result<Result<T::Item, T::Error>, Error>> + Unpin {
        self.router
            .lock()
            .unwrap()
            .forward(&self.addr, msg)
            .compat()
    }

    pub fn call_streaming<T: RpcStreamMessage>(
        &self,
        msg: T,
    ) -> impl Stream<Item = Result<Result<T::Item, T::Error>, Error>> + Unpin {
        self.router
            .lock()
            .unwrap()
            .streaming_forward(&self.addr, msg)
            .compat()
    }
}

impl<T: RpcMessage> RpcEndpoint<T> for Endpoint
where
    T: Send,
{
    type Result = Pin<Box<dyn Future<Output = Result<Result<T::Item, T::Error>, Error>>>>;

    fn send(&self, msg: T) -> Self::Result {
        Endpoint::call(self, msg).boxed_local()
    }
}

pub fn service(addr: &str) -> Endpoint {
    Endpoint {
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

    fn handle(&mut self, caller: &str, msg: T) -> Self::Result {
        self(msg)
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
