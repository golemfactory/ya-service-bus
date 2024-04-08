use actix::{prelude::*, WrapFuture};
use futures::{channel::oneshot, future::Either, prelude::*, FutureExt, SinkExt};
use std::ops::Not;
use std::{collections::HashSet, time::Duration};

use crate::connection::ClientInfo;
use crate::{
    connection::{self, ConnectionRef, LocalRouterHandler, Transport},
    error::ConnectionTimeout,
    Error, RpcRawCall, RpcRawStreamCall,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const RECONNECT_DELAY: Duration = Duration::from_millis(1000);

type RemoteConnection = ConnectionRef<Transport, LocalRouterHandler>;

pub struct RemoteRouter {
    client_info: ClientInfo,
    local_bindings: HashSet<String>,
    pending_calls: Vec<oneshot::Sender<Result<RemoteConnection, ConnectionTimeout>>>,
    connection: Option<RemoteConnection>,
    shutdown_rx: Option<oneshot::Receiver<()>>,
}

impl Actor for RemoteRouter {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        if self.shutdown_rx.is_none() {
            let (tx, rx) = oneshot::channel();
            tokio::task::spawn_local(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = tx.send(());
            });
            self.shutdown_rx.replace(rx);
        }

        if let Some(ref mut rx) = self.shutdown_rx {
            match rx.try_recv() {
                Ok(None) => self.try_connect(ctx),
                _ => log::debug!("(re)connection interrupted"),
            }
        }
    }
}

impl RemoteRouter {
    fn try_connect(&mut self, ctx: &mut <Self as Actor>::Context) {
        // FIXME: this is `SystemService` and as such cannot get input being initialized
        // FIXME: but we need to pass gsb_url from yagnad CLI
        let addr = ya_sb_proto::GsbAddr::default();
        let client_info = self.client_info.clone();

        log::info!("trying to connect to: {}", addr);

        let timeout_h = ctx.run_later(CONNECT_TIMEOUT, |act, ctx| {
            if act.connection.is_none() {
                act.clean_pending_calls(
                    Err(ConnectionTimeout(ya_sb_proto::GsbAddr::default())),
                    ctx,
                );
                log::warn!("connection timed out after {:?}", CONNECT_TIMEOUT);
                ctx.stop();
            }
        });
        let connect_fut = connection::transport(addr.clone())
            .map_err(move |e| Error::ConnectionFail(addr, e))
            .into_actor(self)
            .then(|transport, act, ctx| {
                let transport = match transport {
                    Ok(v) => v,
                    Err(e) => return Either::Left(fut::err(e)),
                };
                let connection =
                    connection::connect_with_handler(client_info, transport, act.handler(ctx));
                act.connection = Some(connection.clone());
                act.clean_pending_calls(Ok(connection.clone()), ctx);
                Either::Right(
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
            .then(move |result: Result<(), Error>, _, ctx| {
                ctx.cancel_future(timeout_h);
                if let Err(e) = result {
                    log::warn!("routing error: {}", e);
                    ctx.run_later(RECONNECT_DELAY, |_, ctx| ctx.stop());
                }
                fut::ready(())
            });

        ctx.spawn(connect_fut);
    }

    fn clean_pending_calls(
        &mut self,
        connection: Result<ConnectionRef<Transport, LocalRouterHandler>, ConnectionTimeout>,
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

    fn connection(&mut self) -> impl Future<Output = Result<RemoteConnection, Error>> + 'static {
        if let Some(c) = &self.connection {
            return future::ok((*c).clone()).left_future();
        }
        log::debug!("wait for connection");
        let (tx, rx) = oneshot::channel();
        self.pending_calls.push(tx);
        rx.map(|r| match r {
            Err(_) => Err(Error::Cancelled),
            Ok(c) => c.map_err(From::from),
        })
        .right_future()
    }

    fn handler(&mut self, ctx: &mut <Self as Actor>::Context) -> LocalRouterHandler {
        let (tx, rx) = oneshot::channel();

        rx.into_actor(self)
            .map(|_, this, ctx| {
                this.connection.as_ref().map(|c| {
                    c.connected().not().then(|| log::warn!("connection lost"));
                });
                // restarts the actor
                ctx.run_later(RECONNECT_DELAY, |_, ctx| ctx.stop());
            })
            .spawn(ctx);

        LocalRouterHandler::new(|| {
            let _ = tx.send(());
        })
    }
}

impl Default for RemoteRouter {
    fn default() -> Self {
        Self {
            connection: Default::default(),
            local_bindings: Default::default(),
            pending_calls: Default::default(),
            client_info: ClientInfo::new("sb-client"),
            shutdown_rx: Default::default(),
        }
    }
}

impl Supervised for RemoteRouter {
    fn restarting(&mut self, _ctx: &mut Self::Context) {
        let _ = self.connection.take();
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

    fn handle(&mut self, msg: UpdateService, _ctx: &mut Self::Context) -> Self::Result {
        match msg {
            UpdateService::Add(service_id) => {
                if let Some(c) = &mut self.connection {
                    Arbiter::current().spawn(c.bind(service_id.clone()).then(|v| async {
                        v.unwrap_or_else(|err| match err {
                            Error::GsbAlreadyRegistered(m) => {
                                log::warn!("already registered: {}", m)
                            }
                            e => log::error!("bind error: {}", e),
                        })
                    }));
                }
                log::trace!("Binding local service '{}'", service_id);
                self.local_bindings.insert(service_id);
            }
            UpdateService::Remove(service_id) => {
                if let Some(c) = &mut self.connection {
                    Arbiter::current().spawn(c.unbind(service_id.clone()).then(|v| async {
                        v.unwrap_or_else(|e| log::error!("unbind error: {}", e))
                    }));
                }
                log::trace!("Unbinding local service '{}'", service_id);
                self.local_bindings.remove(&service_id);
            }
        }
        MessageResult(())
    }
}

impl Handler<RpcRawCall> for RemoteRouter {
    type Result = ActorResponse<Self, Result<Vec<u8>, Error>>;

    fn handle(&mut self, msg: RpcRawCall, _ctx: &mut Self::Context) -> Self::Result {
        ya_packet_trace::packet_trace_maybe!("RemoteRouter::Handler<RpcRawCall>", {
            &ya_packet_trace::try_extract_from_ip_frame(&msg.body)
        });

        ActorResponse::r#async(
            self.connection()
                .and_then(move |connection| {
                    connection.call(msg.caller, msg.addr, msg.body, msg.no_reply)
                })
                .into_actor(self),
        )
    }
}

impl Handler<RpcRawStreamCall> for RemoteRouter {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: RpcRawStreamCall, ctx: &mut Self::Context) -> Self::Result {
        let conn = self.connection();
        let fut = async move {
            let connection = match conn.await {
                Ok(c) => c,
                Err(e) => return log::error!("Remote router connection error: {}", e),
            };

            let reply = msg.reply.sink_map_err(|e| Error::GsbFailure(e.to_string()));
            futures::pin_mut!(reply);

            ya_packet_trace::packet_trace_maybe!("RemoteRouter::Handler<RpcRawStreamCall>", {
                &{ &ya_packet_trace::try_extract_from_ip_frame(&msg.body) }
            });

            let result = SinkExt::send_all(
                &mut reply,
                &mut connection
                    .call_streaming(msg.caller, msg.addr, msg.body)
                    .map(|v| Ok(v)),
            )
            .await;

            if let Err(e) = result {
                log::error!("Remote router RpcRawStreamCall handler error: {}", e);
            }
        };
        ctx.spawn(fut.into_actor(self));
        Ok(())
    }
}
