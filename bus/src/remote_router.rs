use crate::connection::{self, ConnectionRef, LocalRouterHandler, TcpTransport};
use crate::error::Error;
use crate::{Handle, RpcRawCall};
use actix::prelude::*;
use futures::task::{LocalSpawnExt, SpawnExt};
use futures_01::{sync::oneshot, Future, Sink};
use std::alloc::handle_alloc_error;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use ya_sb_proto::codec::{GsbMessage, ProtocolError};

fn gsb_addr() -> std::net::SocketAddr {
    "127.0.0.1:8245".parse().unwrap()
}

pub struct RemoteRouter {
    local_bindings: HashSet<String>,
    pending_calls: Vec<(RpcRawCall, oneshot::Sender<Result<Vec<u8>, Error>>)>,
    connection: Option<ConnectionRef<TcpTransport, LocalRouterHandler>>,
}

impl Actor for RemoteRouter {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.try_connect(ctx)
    }
}

impl RemoteRouter {
    fn try_connect(&mut self, ctx: &mut <Self as Actor>::Context) {
        let connect_fut = connection::tcp(&gsb_addr())
            .map_err(Error::BusConnectionFail)
            .into_actor(self)
            .and_then(|tcp_transport, act, ctx| {
                let connection = connection::connect(tcp_transport);
                act.connection = Some(connection.clone());

                act.clean_pending_calls(connection.clone(), ctx);
                futures_01::future::join_all(
                    act.local_bindings
                        .clone()
                        .into_iter()
                        .map(move |service_id| connection.bind(service_id)),
                )
                .and_then(|v| {
                    log::info!("registed all services");
                    Ok(())
                })
                .into_actor(act)
            });
        ctx.spawn(connect_fut.map_err(|e, _, ctx| {
            log::error!("fail to connect to gsb: {}", e);
            //ctx.stop();
        }));
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
                    Ok(())
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
    fn restarting(&mut self, ctx: &mut Self::Context) {
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
    Remove(String),
}

impl Message for UpdateService {
    type Result = ();
}

impl Handler<UpdateService> for RemoteRouter {
    type Result = MessageResult<UpdateService>;

    fn handle(&mut self, msg: UpdateService, ctx: &mut Self::Context) -> Self::Result {
        match msg {
            UpdateService::Add(service_id) => {
                if let Some(c) = &mut self.connection {
                    Arbiter::spawn(
                        c.bind(service_id.clone())
                            .map_err(|e| log::error!("bind error: {}", e)),
                    )
                }
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

    fn handle(&mut self, msg: RpcRawCall, ctx: &mut Self::Context) -> Self::Result {
        if let Some(c) = &self.connection {
            ActorResponse::r#async(c.call(msg.caller, msg.addr, msg.body).into_actor(self))
        } else {
            let (tx, rx) = oneshot::channel();
            self.pending_calls.push((msg, tx));
            ActorResponse::r#async(rx.flatten().into_actor(self))
        }
    }
}
