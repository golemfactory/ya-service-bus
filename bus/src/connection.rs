use actix::prelude::*;
use futures_01::Sink;

use crate::error::Error;
use crate::local_router::router;
use futures::TryFutureExt;
use futures_01::unsync::oneshot;
use std::collections::{HashMap, VecDeque};
use std::convert::TryInto;
use ya_sb_proto::codec::{GsbMessage, GsbMessageDecoder, GsbMessageEncoder, ProtocolError};
use ya_sb_proto::MessageType;
use ya_sb_proto::{CallReply, CallReplyCode, CallReplyType, RegisterReplyCode, RegisterRequest};

static DEFAULT_URL: &str = "tcp://127.0.0.1:8245";

fn gen_id() -> u64 {
    use rand::Rng;

    let mut rng = rand::thread_rng();

    rng.gen::<u64>() & 0x1f_ff_ff__ff_ff_ff_ffu64
}

struct Connection<W>
where
    W: Sink<SinkItem = GsbMessage, SinkError = ProtocolError>,
{
    writer: actix::io::SinkWrite<W>,
    register_reply: VecDeque<oneshot::Sender<Result<(), Error>>>,
    call_reply: HashMap<String, oneshot::Sender<Result<Vec<u8>, Error>>>,
}

impl<W: 'static> Connection<W>
where
    W: Sink<SinkItem = GsbMessage, SinkError = ProtocolError>,
{
    fn new(w: W, ctx: &mut <Self as Actor>::Context) -> Self {
        Connection {
            writer: io::SinkWrite::new(w, ctx),
            register_reply: Default::default(),
            call_reply: Default::default(),
        }
    }

    fn handle_register_reply(
        &mut self,
        code: RegisterReplyCode,
        msg: String,
        ctx: &mut <Self as Actor>::Context,
    ) {
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
        let mut do_call = router()
            .lock()
            .unwrap()
            .forward_bytes(&address, &caller, data.as_ref())
            .compat()
            .into_actor(self)
            .then(move |r, act: &mut Self, ctx| {
                // TODO: handle write error
                let _ = act.writer.write(GsbMessage::CallReply(match r {
                    Ok(data) => {
                        let code = CallReplyCode::CallReplyOk as i32;
                        let reply_type = CallReplyType::Full as i32;
                        CallReply {
                            request_id,
                            code,
                            reply_type,
                            data,
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
                fut::ok(())
            });
        ctx.spawn(do_call);
    }

    fn handle_reply(
        &mut self,
        request_id: String,
        code: i32,
        reply_type: i32,
        data: Vec<u8>,
        ctx: &mut <Self as Actor>::Context,
    ) -> Result<(), failure::Error> {
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

impl<W: 'static> Actor for Connection<W>
where
    W: Sink<SinkItem = GsbMessage, SinkError = ProtocolError>,
{
    type Context = Context<Self>;
}

fn register_reply_code(code: i32) -> Option<RegisterReplyCode> {
    Some(match code {
        0 => RegisterReplyCode::RegisteredOk,
        400 => RegisterReplyCode::RegisterBadRequest,
        409 => RegisterReplyCode::RegisterConflict,
        _ => return None,
    })
}

impl<W: 'static> StreamHandler<GsbMessage, ProtocolError> for Connection<W>
where
    W: Sink<SinkItem = GsbMessage, SinkError = ProtocolError>,
{
    fn handle(&mut self, item: GsbMessage, ctx: &mut Self::Context) {
        match item {
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

impl<W: 'static> io::WriteHandler<ProtocolError> for Connection<W>
where
    W: Sink<SinkItem = GsbMessage, SinkError = ProtocolError>,
{
    fn error(&mut self, err: ProtocolError, _ctx: &mut Self::Context) -> Running {
        log::error!("protocol error: {}", err);
        Running::Stop
    }
}
