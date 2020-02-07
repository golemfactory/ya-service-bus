use actix::prelude::*;
use futures::channel::oneshot;
use futures::prelude::*;

use crate::error::Error;
use crate::local_router::router;
use crate::{ResponseChunk, RpcRawCall};
use futures::stream::SplitSink;

use futures::StreamExt;
use std::collections::{HashMap, VecDeque};
use std::convert::TryInto;
use std::pin::Pin;
use ya_sb_proto::codec::{GsbMessage, ProtocolError};
use ya_sb_proto::{
    CallReply, CallReplyCode, CallReplyType, CallRequest, RegisterReplyCode, RegisterRequest,
};

fn gen_id() -> u64 {
    use rand::Rng;

    let mut rng = rand::thread_rng();

    rng.gen::<u64>() & 0x1f_ff_ff__ff_ff_ff_ffu64
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
        request_id: String,
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

struct Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin,
    H: CallRequestHandler,
{
    writer: actix::io::SinkWrite<GsbMessage, futures::sink::Buffer<W, GsbMessage>>,
    register_reply: VecDeque<oneshot::Sender<Result<(), Error>>>,
    call_reply: HashMap<String, oneshot::Sender<Result<Vec<u8>, Error>>>,
    handler: H,
}

impl<W: 'static, H: 'static> Unpin for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin,
    H: CallRequestHandler,
{
}

impl<W: 'static, H: 'static> Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin,
    H: CallRequestHandler,
{
    fn new(w: W, handler: H, ctx: &mut <Self as Actor>::Context) -> Self {
        Connection {
            writer: io::SinkWrite::new(w.buffer(256), ctx),
            register_reply: Default::default(),
            call_reply: Default::default(),
            handler,
        }
    }

    fn handle_register_reply(
        &mut self,
        code: RegisterReplyCode,
        msg: String,
        ctx: &mut <Self as Actor>::Context,
    ) {
        log::trace!("got reply: {}", msg);
        if let Some(r) = self.register_reply.pop_front() {
            let _ = match code {
                RegisterReplyCode::RegisteredOk => r.send(Ok(())),
                RegisterReplyCode::RegisterBadRequest => {
                    log::warn!("bad request: {}", msg);
                    r.send(Err(Error::GsbBadRequest(msg)))
                }
                RegisterReplyCode::RegisterConflict => {
                    log::warn!("already registered: {}", msg);
                    r.send(Err(Error::GsbAlreadyRegistered(msg)))
                }
            };
        } else {
            log::error!("unmatched register reply");
            ctx.stop()
        }
    }

    fn handle_call_request(
        &mut self,
        request_id: String,
        caller: String,
        address: String,
        data: Vec<u8>,
        ctx: &mut <Self as Actor>::Context,
    ) {
        log::debug!(
            "handling call from = {}, to = {}, request_id={}, ",
            caller,
            address,
            request_id
        );
        let do_call = self
            .handler
            .do_call(request_id.clone(), caller, address, data)
            .into_actor(self)
            .fold((), move |(), r, act: &mut Self, _ctx| {
                let request_id = request_id.clone();
                // TODO: handle write error
                let _ = act.writer.write(GsbMessage::CallReply(match r {
                    Ok(data) => {
                        let code = CallReplyCode::CallReplyOk as i32;
                        let reply_type = data.reply_type() as i32;
                        CallReply {
                            request_id,
                            code,
                            reply_type,
                            data: data.into_vec(),
                        }
                    }
                    Err(e) => {
                        let code = CallReplyCode::ServiceFailure as i32;
                        let reply_type = Default::default();
                        let data = format!("{}", e).into_bytes();
                        CallReply {
                            request_id,
                            code,
                            reply_type,
                            data,
                        }
                    }
                }));
                //Ok(())
                fut::ready(())
                //fut::ok::<_, Error, _>(())
            });
        //do_call.spawn(ctx);
        ctx.spawn(do_call);
    }

    fn handle_reply(
        &mut self,
        request_id: String,
        code: i32,
        _reply_type: i32,
        data: Vec<u8>,
        ctx: &mut <Self as Actor>::Context,
    ) -> Result<(), Box<dyn std::error::Error>> {
        log::debug!(
            "handling replay for request_id={}, code={}",
            request_id,
            code
        );
        if let Some(r) = self.call_reply.remove(&request_id) {
            // TODO: check error
            let _ = r.send(match code.try_into()? {
                CallReplyCode::CallReplyOk => Ok(data),
                CallReplyCode::CallReplyBadRequest => {
                    Err(Error::GsbBadRequest(String::from_utf8(data)?))
                }
                CallReplyCode::ServiceFailure => Err(Error::GsbFailure(String::from_utf8(data)?)),
            });
        } else {
            log::error!("unmatched call reply");
            ctx.stop()
        }
        Ok(())
    }
}

impl<W: 'static, H: 'static> Actor for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin,
    H: CallRequestHandler,
{
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Self::Context) {
        log::info!("started connection to gsb");
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        log::info!("stopped connection to gsb");
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

impl<W: 'static, H: 'static> StreamHandler<Result<GsbMessage, ProtocolError>> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin,
    H: CallRequestHandler,
{
    fn handle(&mut self, item: Result<GsbMessage, ProtocolError>, ctx: &mut Self::Context) {
        match item.unwrap() {
            GsbMessage::RegisterReply(r) => {
                if let Some(code) = register_reply_code(r.code) {
                    self.handle_register_reply(code, r.message, ctx)
                } else {
                    log::error!("invalid reply code {}", r.code);
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
            m => {
                log::error!("unexpected gsb message: {:?}", m);
                ctx.stop();
            }
        }
    }
}

impl<W: 'static + Unpin, H: CallRequestHandler + 'static> io::WriteHandler<ProtocolError>
    for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError>,
{
    fn error(&mut self, err: ProtocolError, _ctx: &mut Self::Context) -> Running {
        log::error!("protocol error: {}", err);
        Running::Stop
    }
}

impl<W: Unpin + 'static, H: CallRequestHandler + 'static> Handler<RpcRawCall> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError>,
{
    type Result = ActorResponse<Self, Vec<u8>, Error>;

    fn handle(&mut self, msg: RpcRawCall, _ctx: &mut Self::Context) -> Self::Result {
        let (tx, rx) = oneshot::channel();
        let request_id = format!("{}", gen_id());
        let _ = self.call_reply.insert(request_id.clone(), tx);
        let caller = msg.caller;
        let address = msg.addr;
        let data = msg.body;
        log::info!("handling caller: {}, addr:{}", caller, address);
        let _r = self.writer.write(GsbMessage::CallRequest(CallRequest {
            request_id,
            caller,
            address,
            data,
        }));
        ActorResponse::r#async(rx.then(|v| async { v? }).into_actor(self))
    }
}

struct Bind {
    addr: String,
}

impl Message for Bind {
    type Result = Result<(), Error>;
}

impl<W: Unpin + 'static, H: CallRequestHandler + 'static> Handler<Bind> for Connection<W, H>
where
    W: Sink<GsbMessage, Error = ProtocolError>,
{
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: Bind, _ctx: &mut Self::Context) -> Self::Result {
        let (tx, rx) = oneshot::channel();
        self.register_reply.push_back(tx);
        let service_id = msg.addr;
        match self
            .writer
            .write(GsbMessage::RegisterRequest(RegisterRequest { service_id }))
        {
            Ok(()) => (),
            Err(e) => return ActorResponse::reply(Err(Error::GsbFailure(e.to_string()))),
        };

        ActorResponse::r#async(
            async move {
                rx.await??;
                Ok(())
            }
            .into_actor(self),
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
        log::info!("Binding remote service '{}'", addr);
        self.0.send(Bind { addr }).then(|v| async {
            log::trace!("send bind result: {:?}", v);
            v?
        })
    }

    pub fn call(
        &self,
        caller: impl Into<String>,
        addr: impl Into<String>,
        body: impl Into<Vec<u8>>,
    ) -> impl Future<Output = Result<Vec<u8>, Error>> {
        self.0
            .send(RpcRawCall {
                caller: caller.into(),
                addr: addr.into(),
                body: body.into(),
            })
            .then(|v| async { v? })
    }

    pub fn connected(&self) -> bool {
        self.0.connected()
    }
}

pub fn connect<Transport, H: CallRequestHandler + 'static + Default + Unpin>(
    transport: Transport,
) -> ConnectionRef<Transport, H>
where
    Transport: Sink<GsbMessage, Error = ProtocolError>
        + Stream<Item = Result<GsbMessage, ProtocolError>>
        + Unpin
        + 'static,
{
    connect_with_handler(transport, Default::default())
}

pub fn connect_with_handler<Transport, H: CallRequestHandler + 'static>(
    transport: Transport,
    handler: H,
) -> ConnectionRef<Transport, H>
where
    Transport: Sink<GsbMessage, Error = ProtocolError>
        + Stream<Item = Result<GsbMessage, ProtocolError>>
        + Unpin
        + 'static,
{
    let (split_sink, split_stream) = transport.split();
    ConnectionRef(Connection::create(move |ctx| {
        let _h = Connection::add_stream(split_stream, ctx);
        Connection::new(split_sink, handler, ctx)
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
