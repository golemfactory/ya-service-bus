use actix::prelude::*;
use futures_01::future;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::path::PathBuf;
use structopt::StructOpt;
use ya_service_bus::{actix_rpc, Handle, RpcEnvelope, RpcMessage};

const SERVICE_ID: &str = "/local/exe-unit";

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum Command {
    Deploy {},
    Start {
        #[serde(default)]
        args: Vec<String>,
    },
    Run {
        entry_point: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Stop {},
    Transfer {
        from: String,
        to: String,
    },
}

#[derive(Serialize, Deserialize, Debug)]
struct Execute(Vec<Command>);

impl RpcMessage for Execute {
    const ID: &'static str = "yg::exe_unit::execute";
    type Item = ();
    type Error = ();
}

#[derive(Default)]
struct ExeUnit(Option<Handle>);

impl Actor for ExeUnit {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.0 = Some(actix_rpc::bind::<Execute>(
            SERVICE_ID,
            ctx.address().recipient(),
        ))
    }
}

impl Handler<RpcEnvelope<Execute>> for ExeUnit {
    type Result = Result<(), ()>;

    fn handle(&mut self, msg: RpcEnvelope<Execute>, _ctx: &mut Self::Context) -> Self::Result {
        eprintln!("got {:?}", msg.as_ref());
        Ok(())
    }
}

#[derive(StructOpt)]
enum Args {
    /// Starts server that waits for commands on gsb://local/exe-unit
    Server {},
    /// Sends script to gsb://local/exe-unit service
    Client { script: PathBuf },
}

fn main() -> failure::Fallible<()> {
    let args = Args::from_args();
    match args {
        Args::Server { .. } => {
            let sys = System::new("serv");
            let _ = ExeUnit::default().start();
            sys.run()?;
            eprintln!("done");
        }
        Args::Client { script } => {
            let commands: Vec<Command> =
                serde_json::from_reader(OpenOptions::new().read(true).open(script)?)?;
            let mut sys = System::new("cli");

            let result = sys.block_on(future::lazy(|| {
                actix_rpc::service(SERVICE_ID).send(Execute(commands))
            }))?;
            eprintln!("got result: {:?}", result);
        }
    }
    Ok(())
}
