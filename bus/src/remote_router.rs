use crate::connection::{self, ConnectionRef, LocalRouterHandler, TcpTransport};
use crate::error::Error;
use crate::error::Error::GsbFailure;
use crate::{Handle, RpcRawCall, RpcRawStreamCall};
use actix::{prelude::*, WrapFuture};
use futures::{channel::oneshot, prelude::*, FutureExt, SinkExt, StreamExt};
use std::collections::HashSet;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

type RemoteConncetion = ConnectionRef<TcpTransport, LocalRouterHandler>;

pub struct RemoteRouter {
    local_bindings: HashSet<String>,
    pending_calls: Vec<oneshot::Sender<RemoteConncetion>>,
    connection: Option<RemoteConncetion>,
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
                    .and_then(|_| async { Ok(log::debug!("registered all services")) })
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
        log::debug!(
            "got connection activating {} calls",
            self.pending_calls.len()
        );
        for tx in std::mem::replace(&mut self.pending_calls, Default::default()) {
            let connection = connection.clone();
            let send_fut = async move {
                let _v = tx.send(connection);
            }
            .into_actor(self);
            let _ = ctx.spawn(send_fut);
        }
    }

    fn connection(&mut self) -> impl Future<Output = Result<RemoteConncetion, Error>> + 'static {
        if let Some(c) = &self.connection {
            return future::ok((*c).clone()).left_future();
        }
        log::debug!("wait for connection");
        let (tx, rx) = oneshot::channel();
        self.pending_calls.push(tx);
        rx.map_err(From::from).right_future()
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
                log::trace!("Binding local service '{}'", service_id);
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
        ActorResponse::r#async(
            self.connection()
                .and_then(|connection| connection.call(msg.caller, msg.addr, msg.body))
                .into_actor(self),
        )
    }
}

impl Handler<RpcRawStreamCall> for RemoteRouter {
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: RpcRawStreamCall, _ctx: &mut Self::Context) -> Self::Result {
        ActorResponse::r#async(
            self.connection()
                .and_then(|connection| async move {
                    let reply = msg.reply.sink_map_err(|e| Error::GsbFailure(e.to_string()));
                    futures::pin_mut!(reply);

                    let result = SinkExt::send_all(
                        &mut reply,
                        &mut connection
                            .call_streaming(msg.caller, msg.addr, msg.body)
                            .map(|v| Ok(v)),
                    )
                    .await;
                    result
                })
                .into_actor(self),
        )
    }
}
