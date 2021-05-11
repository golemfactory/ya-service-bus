use crate::connection::reader::InputHandler;
use crate::connection::writer::EmptyBufferHandler;
use crate::router::{IdBytes, InstanceConfig, RouterRef};

use actix::prelude::io::WriteHandler;
use actix::prelude::*;
use futures::prelude::*;
use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{FramedRead, FramedWrite};
use ya_sb_proto::codec::{
    GsbMessage, GsbMessageDecoder, GsbMessageEncoder, ProtocolError,
};
use ya_sb_proto::{RegisterReply, RegisterReplyCode};

mod reader;
mod writer;

type TransportWriter<W> = writer::SinkWrite<GsbMessage, W>;
pub type StreamWriter<Output> = FramedWrite<Output, GsbMessageEncoder>;

#[derive(Message)]
#[rtype("()")]
pub struct DropConnection;

pub struct Connection<W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static> {
    config: Arc<InstanceConfig>,
    instance_id: Option<IdBytes>,
    router: RouterRef<W>,
    services: HashSet<String>,
    output: writer::SinkWrite<GsbMessage, W>,
}

impl<W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static> Actor for Connection<W> {
    type Context = Context<Self>;

    fn stopped(&mut self, ctx: &mut Self::Context) {
        self.cleanup(ctx);
    }
}

impl<W> Connection<W>
where
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
{
    fn cleanup(&mut self, ctx: &mut <Self as Actor>::Context) {
        if let Some(instance_id) = self.instance_id.take() {
            log::debug!("cleanup connection");
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
    }
}

pub fn connection<Input: AsyncRead + 'static, Output: AsyncWrite + Unpin>(
    config: Arc<InstanceConfig>,
    router: RouterRef<StreamWriter<Output>>,
    input: Input,
    output: Output,
) -> Addr<Connection<StreamWriter<Output>>> {
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
            output,
        }
    })
}

impl<W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static>
    InputHandler<Result<GsbMessage, ProtocolError>> for Connection<W>
{
    fn handle(
        &mut self,
        item: Result<GsbMessage, ProtocolError>,
        ctx: &mut Context<Self>,
    ) -> Pin<Box<dyn ActorFuture<Output = (), Actor = Self>>> {
        let msg = match item {
            Err(e) => {
                log::error!("protocol error {:?}", e);
                ctx.stop();
                return Box::pin(fut::ready(()));
            }
            Ok(msg) => msg,
        };

        match msg {
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
            GsbMessage::Hello(hello_request) => {
                if self.instance_id.is_some() {
                    log::error!("duplicate hello send");
                    ctx.stop();
                }
                else {
                    let instance_id: IdBytes = hello_request.instance_id.into();
                    self.instance_id = Some(instance_id.clone());
                    return Box::pin(self.router.write().new_connection(instance_id, ctx.address()).into_actor(self))
                }
            }
            m => {
                log::error!("unexpected gsb message: {:?}", m);
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

impl<W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static> Handler<DropConnection>
    for Connection<W>
{
    type Result = ();

    fn handle(&mut self, _: DropConnection, ctx: &mut Self::Context) -> Self::Result {
        log::debug!("forced connection drop");
        self.cleanup(ctx);
        ctx.stop();
    }
}

impl<W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static> WriteHandler<ProtocolError>
    for Connection<W>
{
}

impl<W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static> EmptyBufferHandler
    for Connection<W>
{
    fn buffer_empty(&mut self, _ctx: &mut Self::Context) {
        log::info!("empty buffer");
    }
}
