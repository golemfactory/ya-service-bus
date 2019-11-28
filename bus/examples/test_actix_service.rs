use actix::prelude::*;
use serde::{Deserialize, Serialize};
use ya_service_bus::{actix_rpc, Handle, RpcMessage};

#[derive(Serialize, Deserialize, Debug)]
enum Command {
    Deploy {},
    Start {
        args: Vec<String>,
    },
    Run {
        entry_point: String,
        args: Vec<String>,
    },
    Stop {},
    Transfer {
        src: String,
        dest: String,
    },
}

#[derive(Serialize, Deserialize, Debug)]
struct Execute(Vec<Command>);

impl Message for Execute {
    type Result = Result<(), ()>;
}

impl RpcMessage for Execute {
    const ID: &'static str = "yg::exe_unit::execute";
}

#[derive(Default)]
struct ExeUnit(Option<Handle>);

impl Actor for ExeUnit {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.0 = Some(actix_rpc::bind::<Execute>(
            "/local/exe-unit",
            ctx.address().recipient(),
        ))
    }
}

impl Handler<Execute> for ExeUnit {
    type Result = Result<(), ()>;

    fn handle(&mut self, msg: Execute, _ctx: &mut Self::Context) -> Self::Result {
        eprintln!("got {:?}", msg);
        Ok(())
    }
}

fn main() {}
