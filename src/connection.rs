use actix::prelude::*;
use futures::{
    channel::{mpsc, oneshot},
    prelude::*,
    stream::SplitSink,
};
use std::{
    collections::{HashMap, VecDeque},
    convert::TryInto,
    pin::Pin,
};

use ya_sb_proto::codec::{GsbMessage, ProtocolError};
use ya_sb_proto::{
    BroadcastReplyCode, BroadcastRequest, CallReply, CallReplyCode, CallReplyType, CallRequest,
    RegisterReplyCode, RegisterRequest, SubscribeReplyCode, SubscribeRequest, UnregisterReplyCode,
    UnregisterRequest, UnsubscribeReplyCode, UnsubscribeRequest,
};

use crate::local_router::router;
use crate::Error;
use crate::{ResponseChunk, RpcRawCall, RpcRawStreamCall};
use semver::Version;

fn gen_id() -> u64 {
    use rand::Rng;

    let mut rng = rand::thread_rng();

    rng.gen::<u64>() & 0x1f_ff_ff__ff_ff_ff_ffu64
}

#[derive(Default, Clone)]
#[non_exhaustive]
pub struct ClientInfo {
    pub name: String,
    pub version: Option<Version>,
    pub instance_id: Vec<u8>,
}

pub trait CallRequestHandler {
    type Reply: Stream<Item = Result<ResponseChunk, Error>> + Unpin;

    fn do_call(
        &mut self,
        request_id: String,
        caller: String,
        address: String,
        data: Vec<u8>,
    ) -> Self::Reply;

    fn handle_event(&mut self, caller: String, topic: String, data: Vec<u8>) {
        log::warn!("unhandled gsb event from: {}, to: {}", caller, topic,);
        log::trace!(
            "unhandled gsb event data: {:?}",
            String::from_utf8_lossy(data.as_ref())
        )
    }

    fn on_disconnect(&mut self) {}
}

impl ResponseChunk {
    #[inline]
    fn reply_type(&self) -> CallReplyType {
        match self {
            ResponseChunk::Full(_) => CallReplyType::Full,
            ResponseChunk::Part(_) => CallReplyType::Partial,
        }
    }

    #[inline]
    fn into_vec(self) -> Vec<u8> {
        match self {
            ResponseChunk::Full(v) => v,
            ResponseChunk::Part(v) => v,
        }
    }
}

#[derive(Default)]
pub struct LocalRouterHandler;

impl CallRequestHandler for LocalRouterHandler {
    type Reply = Pin<Box<dyn futures::Stream<Item = Result<ResponseChunk, Error>>>>;

    fn do_call(
        &mut self,
        _request_id: String,
        caller: String,
        address: String,
        data: Vec<u8>,
    ) -> Self::Reply {
        router()
            .lock()
            .unwrap()
            .forward_bytes_local(&address, &caller, data.as_ref())
            .boxed_local()
    }
}

impl<
        R: futures::Stream<Item = Result<ResponseChunk, Error>> + Unpin,
        F: FnMut(String, String, String, Vec<u8>) -> R,
    > CallRequestHandler for F
{
    type Reply = R;

    fn do_call(
        &mut self,
        request_id: String,
        caller: String,
        address: String,
        data: Vec<u8>,
    ) -> Self::Reply {
        self(request_id, caller, address, data)
    }
}

impl<
        R: futures::Stream<Item = Result<ResponseChunk, Error>> + Unpin,
        F1: FnMut(String, String, String, Vec<u8>) -> R,
        F2: FnMut(String, String, Vec<u8>),
    > CallRequestHandler for (F1, F2)
{
    type Reply = R;

    fn do_call(
        &mut self,
        request_id: String,
        caller: String,
        address: String,
        data: Vec<u8>,
    ) -> Self::Reply {
        (self.0)(request_id, caller, address, data)
    }

    fn handle_event(&mut self, caller: String, topic: String, data: Vec<u8>) {
        (self.1)(caller, topic, data)
    }
}

type TransportWriter<W> = actix::io::SinkWrite<GsbMessage, futures::sink::Buffer<W, GsbMessage>>;
type ReplyQueue = VecDeque<oneshot::Sender<Result<(), Error>>>;

struct Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin,
    H: CallRequestHandler,
{
    writer: TransportWriter<W>,
    register_reply: ReplyQueue,
    unregister_reply: ReplyQueue,
    subscribe_reply: ReplyQueue,
    unsubscribe_reply: ReplyQueue,
    call_reply: HashMap<String, mpsc::Sender<Result<ResponseChunk, Error>>>,
    broadcast_reply: ReplyQueue,
    handler: H,
    client_info: ClientInfo,
    server_info: Option<ya_sb_proto::Hello>,
}

impl<W, H> Unpin for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
}

fn handle_reply<Ctx: ActorContext, F: FnOnce() -> Result<(), Error>>(
    cmd_type: &str,
    queue: &mut ReplyQueue,
    ctx: &mut Ctx,
    reply_msg: F,
) {
    if let Some(r) = queue.pop_front() {
        let _ = r.send(reply_msg());
    } else {
        log::error!("unmatched {} reply", cmd_type);
        ctx.stop()
    }
}

impl<W, H> Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    fn new(client_info: ClientInfo, w: W, handler: H, ctx: &mut <Self as Actor>::Context) -> Self {
        Connection {
            writer: io::SinkWrite::new(w.buffer(256), ctx),
            register_reply: Default::default(),
            unregister_reply: Default::default(),
            subscribe_reply: Default::default(),
            unsubscribe_reply: Default::default(),
            call_reply: Default::default(),
            broadcast_reply: Default::default(),
            handler,
            client_info,
            server_info: Default::default(),
        }
    }

    fn handle_unregister_reply(
        &mut self,
        code: UnregisterReplyCode,
        ctx: &mut <Self as Actor>::Context,
    ) {
        handle_reply(
            "unregister",
            &mut self.unregister_reply,
            ctx,
            || match code {
                UnregisterReplyCode::UnregisteredOk => Ok(()),
                UnregisterReplyCode::NotRegistered => {
                    Err(Error::GsbBadRequest("unregister".to_string()))
                }
            },
        )
    }

    fn handle_broadcast_reply(
        &mut self,
        code: BroadcastReplyCode,
        msg: String,
        ctx: &mut <Self as Actor>::Context,
    ) {
        handle_reply("broadcast", &mut self.broadcast_reply, ctx, || match code {
            BroadcastReplyCode::BroadcastOk => Ok(()),
            BroadcastReplyCode::BroadcastBadRequest => Err(Error::GsbBadRequest(msg)),
        })
    }

    fn handle_register_reply(
        &mut self,
        code: RegisterReplyCode,
        msg: String,
        ctx: &mut <Self as Actor>::Context,
    ) {
        handle_reply("register", &mut self.register_reply, ctx, || match code {
            RegisterReplyCode::RegisteredOk => Ok(()),
            RegisterReplyCode::RegisterBadRequest => {
                log::warn!("bad request: {}", msg);
                Err(Error::GsbBadRequest(msg))
            }
            RegisterReplyCode::RegisterConflict => {
                log::warn!("already registered: {}", msg);
                Err(Error::GsbAlreadyRegistered(msg))
            }
        })
    }

    fn handle_subscribe_reply(
        &mut self,
        code: SubscribeReplyCode,
        msg: String,
        ctx: &mut <Self as Actor>::Context,
    ) {
        handle_reply("subscribe", &mut self.subscribe_reply, ctx, || match code {
            SubscribeReplyCode::SubscribedOk => Ok(()),
            SubscribeReplyCode::SubscribeBadRequest => {
                log::warn!("bad request: {}", msg);
                Err(Error::GsbBadRequest(msg))
            }
        })
    }

    fn handle_unsubscribe_reply(
        &mut self,
        code: UnsubscribeReplyCode,
        ctx: &mut <Self as Actor>::Context,
    ) {
        handle_reply(
            "unsubscribe",
            &mut self.unsubscribe_reply,
            ctx,
            || match code {
                UnsubscribeReplyCode::UnsubscribedOk => Ok(()),
                UnsubscribeReplyCode::NotSubscribed => {
                    Err(Error::GsbBadRequest("unsubscribed".to_string()))
                }
            },
        )
    }

    fn handle_call_request(
        &mut self,
        request_id: String,
        caller: String,
        address: String,
        data: Vec<u8>,
        ctx: &mut <Self as Actor>::Context,
    ) {
        log::trace!(
            "handling call from = {}, to = {}, request_id={}, ",
            caller,
            address,
            request_id
        );
        let eos_request_id = request_id.clone();
        let do_call = self
            .handler
            .do_call(request_id.clone(), caller, address, data)
            .into_actor(self)
            .fold(false, move |_got_eos, r, act: &mut Self, _ctx| {
                let request_id = request_id.clone();
                let (got_eos, reply) = match r {
                    Ok(data) => {
                        let code = CallReplyCode::CallReplyOk as i32;
                        let reply_type = data.reply_type() as i32;
                        (
                            reply_type == 0,
                            CallReply {
                                request_id,
                                code,
                                reply_type,
                                data: data.into_vec(),
                            },
                        )
                    }
                    Err(e) => {
                        let code = CallReplyCode::ServiceFailure as i32;
                        let reply_type = Default::default();
                        let data = format!("{}", e).into_bytes();
                        (
                            true,
                            CallReply {
                                request_id,
                                code,
                                reply_type,
                                data,
                            },
                        )
                    }
                };
                // TODO: handle write error
                let _ = act.writer.write(GsbMessage::CallReply(reply));
                fut::ready(got_eos)
            })
            .then(|got_eos, act, _ctx| {
                if !got_eos {
                    let _ = act.writer.write(GsbMessage::CallReply(CallReply {
                        request_id: eos_request_id,
                        code: 0,
                        reply_type: 0,
                        data: Default::default(),
                    }));
                }
                fut::ready(())
            });
        //do_call.spawn(ctx);
        ctx.spawn(do_call);
    }

    fn handle_reply(
        &mut self,
        request_id: String,
        code: i32,
        reply_type: i32,
        data: Vec<u8>,
        ctx: &mut <Self as Actor>::Context,
    ) -> Result<(), Box<dyn std::error::Error>> {
        log::trace!(
            "handling reply for request_id={}, code={}, reply_type={}",
            request_id,
            code,
            reply_type
        );

        let chunk = if reply_type == CallReplyType::Partial as i32 {
            ResponseChunk::Part(data)
        } else {
            ResponseChunk::Full(data)
        };

        let is_full = chunk.is_full();

        if let Some(r) = self.call_reply.get_mut(&request_id) {
            // TODO: check error
            let mut r = (*r).clone();
            let code: CallReplyCode = code.try_into()?;
            let item = match code {
                CallReplyCode::CallReplyOk => Ok(chunk),
                CallReplyCode::CallReplyBadRequest => {
                    Err(Error::GsbBadRequest(String::from_utf8(chunk.into_bytes())?))
                }
                CallReplyCode::ServiceFailure => {
                    Err(Error::GsbFailure(String::from_utf8(chunk.into_bytes())?))
                }
            };
            let _ = ctx.wait(
                async move {
                    let s = r.send(item);
                    s.await
                        .unwrap_or_else(|e| log::warn!("undelivered reply: {}", e))
                }
                .into_actor(self),
            );
        } else {
            log::error!("unmatched call reply");
            ctx.stop()
        }

        if is_full {
            let _ = self.call_reply.remove(&request_id);
        }

        Ok(())
    }
}

impl<W, H> Actor for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.set_mailbox_capacity(256);
        log::info!("started connection to gsb");
        let hello: ya_sb_proto::Hello = ya_sb_proto::Hello {
            name: self.client_info.name.clone(),
            version: self
                .client_info
                .version
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_default(),
            instance_id: self.client_info.instance_id.clone(),
            ..Default::default()
        };

        let _ = self.writer.write(GsbMessage::Hello(hello));
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        log::info!("stopped connection to gsb");
        self.handler.on_disconnect();
    }
}

fn register_reply_code(code: i32) -> Option<RegisterReplyCode> {
    Some(match code {
        0 => RegisterReplyCode::RegisteredOk,
        400 => RegisterReplyCode::RegisterBadRequest,
        409 => RegisterReplyCode::RegisterConflict,
        _ => return None,
    })
}

fn unregister_reply_code(code: i32) -> Option<UnregisterReplyCode> {
    Some(match code {
        0 => UnregisterReplyCode::UnregisteredOk,
        404 => UnregisterReplyCode::NotRegistered,
        _ => return None,
    })
}

fn subscribe_reply_code(code: i32) -> Option<SubscribeReplyCode> {
    Some(match code {
        0 => SubscribeReplyCode::SubscribedOk,
        400 => SubscribeReplyCode::SubscribeBadRequest,
        _ => return None,
    })
}

fn unsubscribe_reply_code(code: i32) -> Option<UnsubscribeReplyCode> {
    Some(match code {
        0 => UnsubscribeReplyCode::UnsubscribedOk,
        404 => UnsubscribeReplyCode::NotSubscribed,
        _ => return None,
    })
}

fn broadcast_reply_code(code: i32) -> Option<BroadcastReplyCode> {
    Some(match code {
        0 => BroadcastReplyCode::BroadcastOk,
        400 => BroadcastReplyCode::BroadcastBadRequest,
        _ => return None,
    })
}

impl<W, H> StreamHandler<Result<GsbMessage, ProtocolError>> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    fn handle(&mut self, item: Result<GsbMessage, ProtocolError>, ctx: &mut Self::Context) {
        if let Err(e) = item.as_ref() {
            log::error!("protocol error {}", e);
            ctx.stop();
            return;
        }

        match item.unwrap() {
            GsbMessage::RegisterReply(r) => {
                if let Some(code) = register_reply_code(r.code) {
                    self.handle_register_reply(code, r.message, ctx)
                } else {
                    log::error!("invalid reply code {}", r.code);
                    ctx.stop();
                }
            }
            GsbMessage::UnregisterReply(r) => {
                if let Some(code) = unregister_reply_code(r.code) {
                    self.handle_unregister_reply(code, ctx)
                } else {
                    log::error!("invalid unregister reply code {}", r.code);
                    ctx.stop();
                }
            }
            GsbMessage::SubscribeReply(r) => {
                if let Some(code) = subscribe_reply_code(r.code) {
                    self.handle_subscribe_reply(code, r.message, ctx)
                } else {
                    log::error!("invalid reply code {}", r.code);
                    ctx.stop();
                }
            }
            GsbMessage::UnsubscribeReply(r) => {
                if let Some(code) = unsubscribe_reply_code(r.code) {
                    self.handle_unsubscribe_reply(code, ctx)
                } else {
                    log::error!("invalid unsubscribe reply code {}", r.code);
                    ctx.stop();
                }
            }
            GsbMessage::BroadcastReply(r) => {
                if let Some(code) = broadcast_reply_code(r.code) {
                    self.handle_broadcast_reply(code, r.message, ctx)
                } else {
                    log::error!("invalid broadcast reply code {}", r.code);
                    ctx.stop();
                }
            }
            GsbMessage::CallRequest(r) => {
                self.handle_call_request(r.request_id, r.caller, r.address, r.data, ctx)
            }
            GsbMessage::CallReply(r) => {
                if let Err(e) = self.handle_reply(r.request_id, r.code, r.reply_type, r.data, ctx) {
                    log::error!("error on call reply processing: {}", e);
                    ctx.stop();
                }
            }
            GsbMessage::BroadcastRequest(r) => {
                self.handler.handle_event(r.caller, r.topic, r.data);
            }
            GsbMessage::Ping(_) => {
                if let Err(e) = self.writer.write(GsbMessage::pong()) {
                    log::error!("error sending pong: {}", e);
                    ctx.stop();
                }
            }
            GsbMessage::Hello(h) => {
                log::debug!("connected with server: {}/{}", h.name, h.version);
                if self.server_info.is_some() {
                    log::error!("invalid packet: {:?}", h);
                    ctx.stop();
                } else {
                    self.server_info = Some(h);
                }
            }
            m => {
                log::error!("unexpected gsb message: {:?}", m);
                ctx.stop();
            }
        }
    }
}

impl<W, H> io::WriteHandler<ProtocolError> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    fn error(&mut self, err: ProtocolError, _ctx: &mut Self::Context) -> Running {
        log::error!("protocol error: {}", err);
        Running::Stop
    }
}

impl<W, H> Handler<RpcRawCall> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    type Result = ActorResponse<Self, Vec<u8>, Error>;

    fn handle(&mut self, msg: RpcRawCall, _ctx: &mut Self::Context) -> Self::Result {
        let (tx, mut rx) = mpsc::channel(1);
        let request_id = format!("{}", gen_id());
        let _ = self.call_reply.insert(request_id.clone(), tx);
        let caller = msg.caller;
        let address = msg.addr;
        let data = msg.body;
        log::trace!("handling caller (rpc): {}, addr:{}", caller, address);
        let _r = self.writer.write(GsbMessage::CallRequest(CallRequest {
            request_id,
            caller,
            address,
            data,
        }));
        let fetch_response = async move {
            match futures::StreamExt::next(&mut rx).await {
                Some(Ok(ResponseChunk::Full(data))) => Ok(data),
                Some(Err(e)) => Err(e),
                Some(Ok(ResponseChunk::Part(_))) => {
                    Err(Error::GsbFailure("streaming response".to_string()))
                }
                None => Err(Error::GsbFailure("unexpected EOS".to_string())),
            }
        };
        ActorResponse::r#async(fetch_response.into_actor(self))
    }
}

impl<W, H> Handler<RpcRawStreamCall> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: RpcRawStreamCall, _ctx: &mut Self::Context) -> Self::Result {
        let request_id = format!("{}", gen_id());
        let rx = msg.reply;
        let _ = self.call_reply.insert(request_id.clone(), rx);
        let caller = msg.caller;
        let address = msg.addr;
        let data = msg.body;
        log::trace!("handling caller (stream): {}, addr:{}", caller, address);
        let _r = self.writer.write(GsbMessage::CallRequest(CallRequest {
            request_id,
            caller,
            address,
            data,
        }));
        ActorResponse::reply(Ok(()))
    }
}

fn send_cmd_async<A: Actor, W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static>(
    writer: &mut TransportWriter<W>,
    queue: &mut VecDeque<oneshot::Sender<Result<(), Error>>>,
    msg: GsbMessage,
) -> ActorResponse<A, (), Error> {
    let (tx, rx) = oneshot::channel();
    queue.push_back(tx);

    if let Err(e) = writer.write(msg) {
        ActorResponse::reply(Err(Error::GsbFailure(format!("no connection: {:?}", e))))
    } else {
        ActorResponse::r#async(fut::wrap_future(async move {
            rx.await.map_err(|_| Error::Cancelled)??;
            Ok(())
        }))
    }
}

struct Bind {
    addr: String,
}

impl Message for Bind {
    type Result = Result<(), Error>;
}

impl<W, H> Handler<Bind> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: Bind, _ctx: &mut Self::Context) -> Self::Result {
        let service_id = msg.addr;
        send_cmd_async(
            &mut self.writer,
            &mut self.register_reply,
            GsbMessage::RegisterRequest(RegisterRequest { service_id }),
        )
    }
}

struct Unbind {
    addr: String,
}

impl Message for Unbind {
    type Result = Result<(), Error>;
}

impl<W, H> Handler<Unbind> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: Unbind, _ctx: &mut Self::Context) -> Self::Result {
        let service_id = msg.addr;
        send_cmd_async(
            &mut self.writer,
            &mut self.unregister_reply,
            GsbMessage::UnregisterRequest(UnregisterRequest { service_id }),
        )
    }
}

struct Subscribe {
    topic: String,
}

impl Message for Subscribe {
    type Result = Result<(), Error>;
}

impl<W, H> Handler<Subscribe> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: Subscribe, _ctx: &mut Self::Context) -> Self::Result {
        let topic = msg.topic;
        send_cmd_async(
            &mut self.writer,
            &mut self.subscribe_reply,
            GsbMessage::SubscribeRequest(SubscribeRequest { topic }),
        )
    }
}

struct Unsubscribe {
    topic: String,
}

impl Message for Unsubscribe {
    type Result = Result<(), Error>;
}

impl<W, H> Handler<Unsubscribe> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: Unsubscribe, _ctx: &mut Self::Context) -> Self::Result {
        let topic = msg.topic;
        send_cmd_async(
            &mut self.writer,
            &mut self.unsubscribe_reply,
            GsbMessage::UnsubscribeRequest(UnsubscribeRequest { topic }),
        )
    }
}

pub struct BcastCall {
    pub caller: String,
    pub topic: String,
    pub body: Vec<u8>,
}

impl Message for BcastCall {
    type Result = Result<(), Error>;
}

impl<W, H> Handler<BcastCall> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
{
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: BcastCall, _ctx: &mut Self::Context) -> Self::Result {
        let caller = msg.caller;
        let topic = msg.topic;
        let data = msg.body;
        send_cmd_async(
            &mut self.writer,
            &mut self.broadcast_reply,
            GsbMessage::BroadcastRequest(BroadcastRequest {
                caller,
                topic,
                data,
            }),
        )
    }
}

pub struct ConnectionRef<
    Transport: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    H: CallRequestHandler + 'static,
>(Addr<Connection<SplitSink<Transport, GsbMessage>, H>>);

impl<
        Transport: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        H: CallRequestHandler + 'static,
    > Unpin for ConnectionRef<Transport, H>
{
}

impl<
        Transport: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        H: CallRequestHandler + 'static,
    > Clone for ConnectionRef<Transport, H>
{
    fn clone(&self) -> Self {
        ConnectionRef(self.0.clone())
    }
}

impl<
        Transport: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        H: CallRequestHandler + Unpin + 'static,
    > ConnectionRef<Transport, H>
{
    pub fn bind(
        &self,
        addr: impl Into<String>,
    ) -> impl Future<Output = Result<(), Error>> + 'static {
        let addr = addr.into();
        log::trace!("Binding remote service '{}'", addr);
        self.0.send(Bind { addr: addr.clone() }).then(|v| async {
            log::trace!("send bind result: {:?}", v);
            v.map_err(|e| Error::from_addr(addr, e))?
        })
    }

    pub fn unbind(
        &self,
        addr: impl Into<String>,
    ) -> impl Future<Output = Result<(), Error>> + 'static {
        let addr = addr.into();
        self.0.send(Unbind { addr: addr.clone() }).then(|v| async {
            log::trace!("send unbind result: {:?}", v);
            v.map_err(|e| Error::from_addr(addr, e))?
        })
    }

    pub fn subscribe(
        &self,
        topic: impl Into<String>,
    ) -> impl Future<Output = Result<(), Error>> + 'static {
        let topic = topic.into();
        let fut = self.0.send(Subscribe {
            topic: topic.clone(),
        });
        async move {
            fut.await
                .map_err(|e| Error::from_addr(format!("subscribing {}", topic).into(), e))?
        }
    }

    pub fn unsubscribe(
        &self,
        topic: impl Into<String>,
    ) -> impl Future<Output = Result<(), Error>> + 'static {
        let topic = topic.into();
        let fut = self.0.send(Unsubscribe {
            topic: topic.clone(),
        });
        async move {
            fut.await
                .map_err(|e| Error::from_addr(format!("unsubscribing {}", topic).into(), e))?
        }
    }

    pub fn broadcast(
        &self,
        caller: impl Into<String>,
        topic: impl Into<String>,
        body: Vec<u8>,
    ) -> impl Future<Output = Result<(), Error>> + 'static {
        let topic = topic.into();
        let fut = self.0.send(BcastCall {
            caller: caller.into(),
            topic: topic.clone(),
            body,
        });
        async move {
            fut.await
                .map_err(|e| Error::from_addr(format!("broadcasting {}", topic).into(), e))?
        }
    }

    pub fn call(
        &self,
        caller: impl Into<String>,
        addr: impl Into<String>,
        body: impl Into<Vec<u8>>,
    ) -> impl Future<Output = Result<Vec<u8>, Error>> {
        let addr = addr.into();
        self.0
            .send(RpcRawCall {
                caller: caller.into(),
                addr: addr.clone(),
                body: body.into(),
            })
            .then(|v| async { v.map_err(|e| Error::from_addr(addr, e))? })
    }

    pub fn call_streaming(
        &self,
        caller: impl Into<String>,
        addr: impl Into<String>,
        body: impl Into<Vec<u8>>,
    ) -> impl Stream<Item = Result<ResponseChunk, Error>> {
        let addr = addr.into();
        let (tx, rx) = futures::channel::mpsc::channel(16);

        let args = RpcRawStreamCall {
            caller: caller.into(),
            addr: addr.clone(),
            body: body.into(),
            reply: tx.clone(),
        };
        let connection = self.0.clone();
        let _ = Arbiter::spawn(async move {
            let mut tx = tx;
            match connection.send(args).await {
                Ok(Ok(())) => (),
                Ok(Err(e)) => {
                    tx.send(Err(e))
                        .await
                        .unwrap_or_else(|e| log::error!("fail: {}", e));
                }
                Err(e) => {
                    tx.send(Err(Error::from_addr(addr, e)))
                        .await
                        .unwrap_or_else(|e| log::error!("fail: {}", e));
                }
            }
        });
        rx
    }

    pub fn connected(&self) -> bool {
        self.0.connected()
    }
}

pub fn connect<Transport, H>(
    client_info: ClientInfo,
    transport: Transport,
) -> ConnectionRef<Transport, H>
where
    Transport: Sink<GsbMessage, Error = ProtocolError>
        + Stream<Item = Result<GsbMessage, ProtocolError>>
        + Unpin
        + 'static,
    H: CallRequestHandler + 'static + Default + Unpin,
{
    connect_with_handler(client_info, transport, Default::default())
}

pub fn connect_with_handler<Transport, H>(
    client_info: ClientInfo,
    transport: Transport,
    handler: H,
) -> ConnectionRef<Transport, H>
where
    Transport: Sink<GsbMessage, Error = ProtocolError>
        + Stream<Item = Result<GsbMessage, ProtocolError>>
        + Unpin
        + 'static,
    H: CallRequestHandler + 'static,
{
    let (split_sink, split_stream) = transport.split();
    ConnectionRef(Connection::create(move |ctx| {
        let _h = Connection::add_stream(split_stream, ctx);
        Connection::new(client_info, split_sink, handler, ctx)
    }))
}

pub type TcpTransport =
    tokio_util::codec::Framed<tokio::net::TcpStream, ya_sb_proto::codec::GsbMessageCodec>;

pub async fn tcp(addr: std::net::SocketAddr) -> Result<TcpTransport, std::io::Error> {
    let s = tokio::net::TcpStream::connect(addr).await?;
    Ok(tokio_util::codec::Framed::new(
        s,
        ya_sb_proto::codec::GsbMessageCodec::default(),
    ))
}
