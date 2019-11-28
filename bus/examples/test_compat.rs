use actix::prelude::*;
use futures::{
    compat::Future01CompatExt,
    future::{FutureExt, TryFutureExt},
};
use serde::{Deserialize, Serialize};
use std::io;
use ya_service_bus::{actix_rpc, Handle};

#[derive(Default)]
struct Server(Option<Handle>);

impl Actor for Server {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.0 = Some(actix_rpc::bind("/local/ping", ctx.address().recipient()))
    }
}

#[derive(Serialize, Deserialize)]
struct Ping(String);

impl Message for Ping {
    type Result = String;
}

impl Handler<Ping> for Server {
    type Result = MessageResult<Ping>;

    fn handle(&mut self, msg: Ping, _ctx: &mut Self::Context) -> Self::Result {
        eprintln!("got ping");
        MessageResult(msg.0.into())
    }
}

async fn start_server() {
    let server = Server::default().start();

    let resp = server.send(Ping("test01".into())).compat().await.unwrap();
    eprintln!("resp = {}", resp);
}

fn main() -> io::Result<()> {
    actix::run(|| start_server().unit_error().boxed_local().compat())
}
