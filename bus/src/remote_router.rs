use crate::connection::{self, ConnectionRef, LocalRouterHandler, TcpTransport};
use crate::error::Error;
use crate::{Handle, RpcRawCall, RpcRawStreamCall};
use actix::{prelude::*, WrapFuture};
use futures::{channel::oneshot, prelude::*};
use std::collections::HashSet;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

pub struct RemoteRouter {
    local_bindings: HashSet<String>,
    pending_calls: Vec<(RpcRawCall, oneshot::Sender<Result<Vec<u8>, Error>>)>,
    connection: Option<ConnectionRef<TcpTransport, LocalRouterHandler>>,
}

impl Actor for RemoteRouter {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.try_connect(ctx);
        let _ = ctx.run_later(CONNECT_TIMEOUT, |act, _ctx| {
            if act.connection.is_none() {
                act.pending_calls.clear();
            }
        });
    }
}

impl RemoteRouter {
    fn try_connect(&mut self, ctx: &mut <Self as Actor>::Context) {
        let addr = ya_sb_proto::gsb_addr();
        let connect_fut = connection::tcp(addr)
            .map_err(move |e| Error::BusConnectionFail(addr, e))
            .into_actor(self)
            .then(|tcp_transport, act, ctx| {
                let tcp_transport = match tcp_transport {
                    Ok(v) => v,
                    Err(e) => return fut::Either::Left(fut::err(e)),
                };
                let connection = connection::connect(tcp_transport);
                act.connection = Some(connection.clone());
                act.clean_pending_calls(connection.clone(), ctx);
                fut::Either::Right(
                    future::try_join_all(
                        act.local_bindings
                            .clone()
                            .into_iter()
                            .map(move |service_id| connection.bind(service_id)),
                    )
                    .and_then(|_| async { Ok(log::info!("registered all services")) })
                    .into_actor(act),
                )
            })
            .then(|v: Result<(), Error>, _, _| {
                if let Err(e) = v {
                    log::warn!("routing error: {}", e);
                }
                fut::ready(())
            });

        ctx.spawn(connect_fut);
    }

    fn clean_pending_calls(
        &mut self,
        connection: ConnectionRef<TcpTransport, LocalRouterHandler>,
        ctx: &mut <Self as Actor>::Context,
    ) {
        for (msg, tx) in std::mem::replace(&mut self.pending_calls, Default::default()) {
            let send_fut = connection
                .call(msg.caller, msg.addr, msg.body)
                .then(|r| {
                    let _ = tx.send(r);
                    future::ready(())
                })
                .into_actor(self);
            ctx.spawn(send_fut);
        }
    }
}

impl Default for RemoteRouter {
    fn default() -> Self {
        Self {
            connection: Default::default(),
            local_bindings: Default::default(),
            pending_calls: Default::default(),
        }
    }
}

impl Supervised for RemoteRouter {
    fn restarting(&mut self, _ctx: &mut Self::Context) {
        if let Some(c) = self.connection.take() {
            if c.connected() {
                self.connection = Some(c)
            } else {
                log::error!("lost connection");
            }
        }
    }
}

impl SystemService for RemoteRouter {}

pub enum UpdateService {
    Add(String),
    #[allow(dead_code)]
    Remove(String),
}

impl Message for UpdateService {
    type Result = ();
}

impl Handler<UpdateService> for RemoteRouter {
    type Result = MessageResult<UpdateService>;

    fn handle(&mut self, msg: UpdateService, _ctx: &mut Self::Context) -> Self::Result {
        match msg {
            UpdateService::Add(service_id) => {
                if let Some(c) = &mut self.connection {
                    Arbiter::spawn(
                        c.bind(service_id.clone()).then(|v| async {
                            v.unwrap_or_else(|e| log::error!("bind error: {}", e))
                        }),
                    )
                }
                log::info!("Binding local service '{}'", service_id);
                self.local_bindings.insert(service_id);
            }
            UpdateService::Remove(service_id) => {
                self.local_bindings.remove(&service_id);
            }
        }
        MessageResult(())
    }
}

impl Handler<RpcRawCall> for RemoteRouter {
    type Result = ActorResponse<Self, Vec<u8>, Error>;

    fn handle(&mut self, msg: RpcRawCall, _ctx: &mut Self::Context) -> Self::Result {
        if let Some(c) = &self.connection {
            ActorResponse::r#async(c.call(msg.caller, msg.addr, msg.body).into_actor(self))
        } else {
            let (tx, rx) = oneshot::channel();
            self.pending_calls.push((msg, tx));
            ActorResponse::r#async(rx.then(|v| async { v? }).into_actor(self))
        }
    }
}

impl Handler<RpcRawStreamCall> for RemoteRouter {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: RpcRawStreamCall, ctx: &mut Self::Context) -> Self::Result {
        unimplemented!()
    }
}
