#![allow(clippy::map_entry)]

use std::collections::{BTreeMap, HashSet};
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use actix::prelude::io::WriteHandler;
use actix::prelude::*;
use futures::channel::oneshot;
use futures::future::LocalBoxFuture;
use futures::prelude::*;
use futures::FutureExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{FramedRead, FramedWrite};

use ya_sb_proto::codec::{GsbMessage, GsbMessageDecoder, GsbMessageEncoder, ProtocolError};
use ya_sb_proto::*;
use ya_sb_util::writer;
use ya_sb_util::writer::EmptyBufferHandler;

use crate::connection::reader::InputHandler;
use crate::router::{IdBytes, InstanceConfig, RouterRef};

mod reader;

pub type StreamWriter<Output> = FramedWrite<Output, GsbMessageEncoder>;

#[derive(Message)]
#[rtype("()")]
pub struct DropConnection;

#[derive(Message)]
#[rtype("Result<(), oneshot::Canceled>")]
pub struct ForwardCallResponse {
    call_reply: CallReply,
}

#[derive(Message)]
#[rtype("Result<(), oneshot::Canceled>")]
pub struct ForwardCallRequest {
    call_request: CallRequest,
    reply_to: Recipient<ForwardCallResponse>,
}

pub struct Connection<
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    ConnInfo: Debug + Unpin + 'static,
> {
    config: Arc<InstanceConfig>,
    instance_id: Option<IdBytes>,
    router: RouterRef<W, ConnInfo>,
    services: HashSet<String>,
    output: writer::SinkWrite<GsbMessage, W>,
    reply_map: BTreeMap<String, Recipient<ForwardCallResponse>>,
    hold_queue: Vec<(GsbMessage, oneshot::Sender<()>)>,
    topic_map: BTreeMap<String, SpawnHandle>,
    conn_info: ConnInfo,
    last_packet: Instant,
}

impl<
        W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        ConnInfo: Debug + Unpin + 'static,
    > Actor for Connection<W, ConnInfo>
{
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        log::debug!("[{:?}] connection started", self.conn_info);
        let _ = ctx.run_interval(self.config.ping_interval(), move |act, ctx| {
            let since_last = Instant::now().duration_since(act.last_packet);
            if since_last > act.config.ping_interval() / 2 {
                if since_last > act.config.ping_timeout() {
                    log::warn!(
                        "[{:?}] no data for {:?} killing connection",
                        act.conn_info,
                        since_last
                    );
                    ctx.stop();
                    return;
                }
                log::debug!(
                    "[{:?}] no data for: {:?}, sending ping (buffer={})",
                    act.conn_info,
                    since_last,
                    act.output.buffer_len()
                );
                act.output.write(GsbMessage::Ping(Default::default()));
            }
        });
    }

    fn stopped(&mut self, ctx: &mut Self::Context) {
        self.cleanup(ctx);
    }
}

impl<W, ConnInfo> Connection<W, ConnInfo>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    ConnInfo: Debug + Unpin + 'static,
{
    fn cleanup(&mut self, ctx: &mut <Self as Actor>::Context) {
        if let Some(instance_id) = self.instance_id.take() {
            log::debug!("[{:?}] cleanup connection", self.conn_info);
            let addr = ctx.address();
            let mut router = self.router.write();
            for service_id in self.services.drain() {
                router.unregister_service(&service_id, &addr)
            }
            router.remove_connection(instance_id, &addr);
        }
    }

    fn send_reply(&mut self, reply: impl Into<GsbMessage>, _ctx: &mut <Self as Actor>::Context) {
        self.output.write(reply.into());
        log::debug!(
            "[{:?}] reply queued. size={}",
            self.conn_info,
            self.output.buffer_len()
        )
    }

    fn handle_call_request(
        &mut self,
        call_request: CallRequest,
        ctx: &mut <Self as Actor>::Context,
    ) -> impl Future<Output = Result<(), CallReply>> + 'static {
        let request_id = call_request.request_id.clone();

        if let Some(dst) = { self.router.read().resolve_node(&call_request.address) } {
            let reply_to = ctx.address().recipient();
            let msg = ForwardCallRequest {
                call_request,
                reply_to,
            };
            dst.send(msg)
                .timeout(self.config.forward_timeout())
                .map(move |r| {
                    let error = match r {
                        Ok(Ok(())) => None,
                        Ok(Err(_)) => Some("request canceled"),
                        Err(MailboxError::Closed) => Some("lost connection"),
                        Err(MailboxError::Timeout) => Some("stalled connection"),
                    };

                    if let Some(err_msg) = error {
                        let mut reply = CallReply {
                            request_id,
                            data: err_msg.as_bytes().to_vec(),
                            ..Default::default()
                        };
                        reply.set_code(CallReplyCode::ServiceFailure);
                        reply.set_reply_type(CallReplyType::Full);
                        Err(reply)
                    } else {
                        Ok(())
                    }
                })
                .left_future()
        } else {
            let mut reply = CallReply {
                request_id,
                ..Default::default()
            };
            reply.set_code(CallReplyCode::CallReplyBadRequest);
            reply.set_reply_type(CallReplyType::Full);
            reply.data = "endpoint address not found".as_bytes().to_vec();

            future::err(reply).right_future()
        }
    }

    fn handle_call_reply(
        &mut self,
        call_reply: CallReply,
        _ctx: &mut <Self as Actor>::Context,
    ) -> impl Future<Output = Result<(), String>> + 'static {
        if let Some(dst) = match call_reply.reply_type() {
            CallReplyType::Full => self.reply_map.remove(&call_reply.request_id),
            CallReplyType::Partial => self.reply_map.get(&call_reply.request_id).map(Clone::clone),
        } {
            let request_id = call_reply.request_id.clone();
            dst.send(ForwardCallResponse { call_reply })
                .map(move |v| match v {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(_)) => Err(format!("unable to send reply {}, canceled", request_id)),
                    Err(e) => Err(format!("unable to send reply {:?},", e)),
                })
                .left_future()
        } else {
            future::err(format!(
                "received unmatched reply {}",
                call_reply.request_id
            ))
            .right_future()
        }
    }

    fn send_message(
        &mut self,
        msg: GsbMessage,
        _ctx: &mut <Self as Actor>::Context,
    ) -> LocalBoxFuture<'static, Result<(), oneshot::Canceled>> {
        if self.output.buffer_len() < self.config.high_buffer_mark() && self.hold_queue.is_empty() {
            self.output.write(msg);
            log::debug!("[{:?}] buffer {}", self.conn_info, self.output.buffer_len());
            future::ok(()).boxed_local()
        } else {
            let (tx, rx) = oneshot::channel();
            self.hold_queue.push((msg, tx));
            log::debug!("[{:?}] queue {}", self.conn_info, self.hold_queue.len());
            rx.boxed_local()
        }
    }
}

pub fn connection<
    Input: AsyncRead + 'static,
    Output: AsyncWrite + Unpin,
    ConnInfo: Debug + Unpin + 'static,
>(
    config: Arc<InstanceConfig>,
    router: RouterRef<StreamWriter<Output>, ConnInfo>,
    conn_info: ConnInfo,
    input: Input,
    output: Output,
) -> Addr<Connection<StreamWriter<Output>, ConnInfo>> {
    let reader = FramedRead::new(input, GsbMessageDecoder::default());
    let writer = FramedWrite::new(output, GsbMessageEncoder::default());
    Connection::create(move |ctx| {
        let output = writer::SinkWrite::new(writer, ctx);
        let _ = Connection::add_stream(reader, ctx);
        Connection {
            instance_id: None,
            router,
            config,
            services: Default::default(),
            hold_queue: Default::default(),
            reply_map: Default::default(),
            topic_map: Default::default(),
            conn_info,
            output,
            last_packet: Instant::now(),
        }
    })
}

impl<
        W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        ConnInfo: Debug + Unpin + 'static,
    > InputHandler<Result<GsbMessage, ProtocolError>> for Connection<W, ConnInfo>
{
    fn handle(
        &mut self,
        item: Result<GsbMessage, ProtocolError>,
        ctx: &mut Context<Self>,
    ) -> Pin<Box<dyn ActorFuture<Self, Output = ()>>> {
        self.last_packet = Instant::now();

        let msg = match item {
            Err(e) => {
                log::error!("[{:?}] protocol error {:?}", self.conn_info, e);
                ctx.stop();
                return Box::pin(fut::ready(()));
            }
            Ok(msg) => msg,
        };

        match msg {
            GsbMessage::CallRequest(call_request) => {
                return Box::pin(
                    self.handle_call_request(call_request, ctx)
                        .into_actor(self)
                        .then(|r, act, ctx| {
                            if let Err(error_reply) = r {
                                act.send_reply(error_reply, ctx);
                            }
                            fut::ready(())
                        }),
                )
            }
            GsbMessage::CallReply(call_reply) => {
                return Box::pin(
                    self.handle_call_reply(call_reply, ctx)
                        .into_actor(self)
                        .then(|r, act, _ctx| {
                            if let Err(msg) = r {
                                log::warn!("[{:?}] {}", act.conn_info, msg);
                            }
                            fut::ready(())
                        }),
                )
            }
            GsbMessage::RegisterRequest(register_request) => {
                let me = ctx.address();
                let service_id = register_request.service_id;
                let registered = { self.router.write().register_service(service_id.clone(), me) };
                let mut reply = RegisterReply::default();
                if registered {
                    self.services.insert(service_id);
                } else {
                    reply.set_code(RegisterReplyCode::RegisterConflict)
                }
                self.send_reply(reply, ctx);
            }

            GsbMessage::SubscribeRequest(subscribe_request) => {
                let topic_id = subscribe_request.topic;
                let mut reply = SubscribeReply::default();
                if self.topic_map.contains_key(&topic_id) {
                    reply.set_code(SubscribeReplyCode::SubscribeBadRequest);
                    reply.message = "topic already registered".to_string();
                } else {
                    let rx = self.router.write().subscribe_topic(topic_id.clone());
                    let handle = ctx.spawn(fut::wrap_stream(rx).fold(
                        (),
                        |_, request, act: &mut Self, ctx| {
                            log::debug!("[{:?}] new item", act.conn_info);
                            match request {
                                Ok(broadcast_request) => act.send_message(
                                    GsbMessage::BroadcastRequest(broadcast_request),
                                    ctx,
                                ),
                                Err(_e) => {
                                    log::warn!("[{:?}] failed to recv broadcast", act.conn_info);
                                    future::ok(()).boxed_local()
                                }
                            }
                            .into_actor(act)
                            .then(|r, act, _ctx| {
                                if r.is_err() {
                                    log::warn!("[{:?}] broadcast forward dropped", act.conn_info);
                                } else {
                                    log::debug!("[{:?}] broadcast forwarded", act.conn_info);
                                }
                                fut::ready(())
                            })
                        },
                    ));
                    self.topic_map.insert(topic_id, handle);
                }

                ActorFutureExt::then(fut::ready(()), |(), act: &mut Self, ctx| {
                    let _ =
                        act.send_reply(GsbMessage::SubscribeReply(SubscribeReply::default()), ctx);
                    fut::ready(())
                })
                .spawn(ctx);
            }

            GsbMessage::UnsubscribeRequest(unsubscribe_request) => {
                let mut reply = UnsubscribeReply::default();
                log::debug!(
                    "[{:?}] unsubscribe {}",
                    self.conn_info,
                    unsubscribe_request.topic
                );
                if let Some(handle) = self.topic_map.remove(&unsubscribe_request.topic) {
                    ctx.cancel_future(handle);
                } else {
                    reply.set_code(UnsubscribeReplyCode::NotSubscribed);
                }
                self.send_reply(GsbMessage::UnsubscribeReply(reply), ctx);
            }

            GsbMessage::BroadcastRequest(broadcast_request) => {
                let reply = BroadcastReply::default();
                if let Some(sender) = { self.router.read().find_topic(&broadcast_request.topic) } {
                    log::debug!(
                        "[{:?}] sending bcast to {} receivers",
                        self.conn_info,
                        sender.receiver_count()
                    );
                    let _ = sender.send(broadcast_request);
                }
                self.send_reply(GsbMessage::BroadcastReply(reply), ctx);
            }
            GsbMessage::Hello(hello_request) => {
                if self.instance_id.is_some() {
                    log::error!("[{:?}] duplicate hello send", self.conn_info);
                    ctx.stop();
                } else {
                    let instance_id: IdBytes = hello_request.instance_id.into();
                    self.instance_id = Some(instance_id.clone());
                    log::debug!(
                        "[{:?}] connection initialized peer {}/{}",
                        self.conn_info,
                        hello_request.name,
                        hello_request.version
                    );
                    return Box::pin(
                        self.router
                            .write()
                            .new_connection(instance_id, ctx.address())
                            .into_actor(self),
                    );
                }
            }
            GsbMessage::Ping(_) => {
                self.send_reply(GsbMessage::Ping(Default::default()), ctx);
            }
            GsbMessage::Pong(_) => {
                log::debug!("[{:?}] pong recv", self.conn_info);
            }
            m => {
                log::error!("[{:?}] unexpected gsb message: {:?}", self.conn_info, m);
                ctx.stop();
            }
        }
        Box::pin(fut::ready(()))
    }

    fn started(&mut self, _ctx: &mut Self::Context) {
        let hello = self.config.hello();
        let _ = self.output.write(GsbMessage::Hello(hello));
    }
}

impl<
        W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        ConnInfo: Debug + Unpin + 'static,
    > Handler<DropConnection> for Connection<W, ConnInfo>
{
    type Result = ();

    fn handle(&mut self, _: DropConnection, ctx: &mut Self::Context) -> Self::Result {
        log::debug!("[{:?}] forced connection drop", self.conn_info);
        self.cleanup(ctx);
        ctx.stop();
    }
}

impl<
        W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        ConnInfo: Debug + Unpin + 'static,
    > WriteHandler<ProtocolError> for Connection<W, ConnInfo>
{
}

impl<
        W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        ConnInfo: Debug + Unpin + 'static,
    > EmptyBufferHandler for Connection<W, ConnInfo>
{
    fn buffer_empty(&mut self, _ctx: &mut Self::Context) {
        if self.hold_queue.is_empty() {
            return;
        }

        log::debug!("[{:?}] empty buffer", self.conn_info);
        for (msg, tx) in self
            .hold_queue
            .drain(..)
            .filter(|(_msg, tx)| !tx.is_canceled())
            .take(self.config.high_buffer_mark())
        {
            self.output.write(msg);
            if tx.send(()).is_err() {
                log::error!("[{:?}] failed to notify sender", self.conn_info);
            }
        }
        log::debug!(
            "[{:?}] on empty buffer, filled {}",
            self.conn_info,
            self.output.buffer_len()
        );
    }
}

impl<S, ConnInfo> Handler<ForwardCallResponse> for Connection<S, ConnInfo>
where
    S: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    ConnInfo: Debug + Unpin + 'static,
{
    type Result = ResponseFuture<Result<(), oneshot::Canceled>>;

    fn handle(&mut self, msg: ForwardCallResponse, ctx: &mut Self::Context) -> Self::Result {
        self.send_message(GsbMessage::CallReply(msg.call_reply), ctx)
    }
}

impl<S, ConnInfo> Handler<ForwardCallRequest> for Connection<S, ConnInfo>
where
    S: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    ConnInfo: Debug + Unpin + 'static,
{
    type Result = ResponseFuture<Result<(), oneshot::Canceled>>;

    fn handle(&mut self, msg: ForwardCallRequest, ctx: &mut Self::Context) -> Self::Result {
        if self
            .reply_map
            .insert(msg.call_request.request_id.clone(), msg.reply_to)
            .is_some()
        {
            log::warn!(
                "[{:?}] duplicate message request id forwarded {}",
                self.conn_info,
                msg.call_request.request_id
            );
        }
        self.send_message(GsbMessage::CallRequest(msg.call_request), ctx)
    }
}
