use actix::prelude::*;
use failure::_core::time::Duration;
use futures::{FutureExt, TryFutureExt};
use futures_01::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::path::PathBuf;
use structopt::StructOpt;
use ya_service_bus::{actix_rpc, send_untyped, Handle, RpcEnvelope, RpcMessage};

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
    const ID: &'static str = "execute";
    type Item = String;
    type Error = String;
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
    type Result = Result<String, String>;

    fn handle(&mut self, msg: RpcEnvelope<Execute>, _ctx: &mut Self::Context) -> Self::Result {
        eprintln!("got {:?}", msg.as_ref());
        Ok(format!("{:?}", msg.into_inner()))
    }
}

#[derive(StructOpt)]
enum Args {
    /// Starts server that waits for commands on gsb://local/exe-unit
    Server {},
    /// Sends script to gsb://local/exe-unit service
    Client {
        script: PathBuf,
    },
    Local {
        script: PathBuf,
    },
    LocalRaw {
        script: PathBuf,
    },
}

fn run_script(script: PathBuf) -> impl Future<Item = String, Error = failure::Error> {
    (|| -> Result<_, std::io::Error> {
        let commands: Vec<Command> =
            serde_json::from_reader(OpenOptions::new().read(true).open(script)?)?;
        Ok(commands)
    })()
    .into_future()
    .from_err()
    .and_then(|commands| {
        actix_rpc::service(SERVICE_ID)
            .send(Execute(commands))
            .from_err()
            .and_then(|v| v.map_err(|e| failure::err_msg(e)))
    })
}

async fn run_script_raw(script: PathBuf) -> Result<Result<String, String>, failure::Error> {
    let bytes = rmp_serde::to_vec(&serde_json::from_slice::<Vec<Command>>(
        std::fs::read(script)?.as_slice(),
    )?)?;

    Ok(rmp_serde::from_slice(
        send_untyped(&format!("{}/{}", SERVICE_ID, Execute::ID), bytes.as_ref())
            .await?
            .as_slice(),
    )?)
}

fn main() -> failure::Fallible<()> {
    let mut sys = System::new("test");
    let args = Args::from_args();
    match args {
        Args::Server { .. } => {
            let _ = ExeUnit::default().start();
            sys.run()?;
            eprintln!("done");
        }
        Args::Client { script } => {
            let result = sys.block_on(run_script(script))?;
            eprintln!("got result: {:?}", result);
        }
        Args::Local { script } => {
            let timer = tokio_timer::Timer::default();
            let _ = ExeUnit::default().start();
            let sleep = timer.sleep(Duration::from_millis(500));

            let result = sys.block_on(sleep.from_err().and_then(|_| run_script(script)))?;
            eprintln!("got result: {:?}", result);
        }
        Args::LocalRaw { script } => {
            let timer = tokio_timer::Timer::default();
            let _ = ExeUnit::default().start();
            let sleep = timer.sleep(Duration::from_millis(500));

            let result = sys.block_on(
                sleep
                    .from_err()
                    .and_then(|_| run_script_raw(script).boxed_local().compat()),
            )?;
            eprintln!("got result: {:?}", result);
        }
    }
    Ok(())
}
