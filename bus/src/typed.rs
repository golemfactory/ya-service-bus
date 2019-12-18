use crate::error::Error;
use crate::local_router::{router, Router};
use crate::{
    Handle, RpcEndpoint, RpcEnvelope, RpcHandler, RpcMessage, RpcStreamHandler, RpcStreamMessage,
};
use actix::Message;
use failure::_core::marker::PhantomData;
use futures::compat::{Compat01As03, Future01CompatExt};
use futures::{Future, FutureExt};
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
    unimplemented!()
}

#[derive(Clone)]
struct Forward {
    router: Arc<Mutex<Router>>,
    addr: String,
}

impl<T: RpcMessage> RpcEndpoint<T> for Forward
where
    T: Send,
{
    type Result = Pin<Box<dyn Future<Output = Result<Result<T::Item, T::Error>, Error>>>>;

    fn send(&self, msg: T) -> Self::Result {
        self.router
            .lock()
            .unwrap()
            .forward(&self.addr, msg)
            .compat()
            .boxed_local()
    }
}

pub fn service<T: RpcMessage>(addr: &str) -> impl RpcEndpoint<T> {
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

    fn handle(&mut self, caller: &str, msg: T) -> Self::Result {
        self(msg)
    }
}
