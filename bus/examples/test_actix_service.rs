use actix::prelude::*;
use failure::_core::time::Duration;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::path::PathBuf;
use structopt::StructOpt;
use ya_service_bus::{actix_rpc, untyped, Handle, RpcEnvelope, RpcMessage};

#[derive(Serialize, Deserialize)]
struct Ping(String);

impl RpcMessage for Ping {
    const ID: &'static str = "ping";
    type Item = String;
    type Error = ();
}

const SERVICE_ID: &str = "/local/exeunit";

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
    Ping {
        dst: String,
        msg: String,
    },
}

fn run_script(script: PathBuf) -> impl Future<Output = Result<String, failure::Error>> {
    async move {
        let commands: Vec<Command> =
            serde_json::from_reader(OpenOptions::new().read(true).open(script)?)?;
        let result = actix_rpc::service(SERVICE_ID)
            .send(Execute(commands))
            .await?;
        result.map_err(|e| failure::err_msg(e))
    }
}

async fn run_script_raw(script: PathBuf) -> Result<Result<String, String>, failure::Error> {
    let bytes = rmp_serde::to_vec(&serde_json::from_slice::<Vec<Command>>(
        std::fs::read(script)?.as_slice(),
    )?)?;

    Ok(rmp_serde::from_slice(
        untyped::send(
            &format!("{}/{}", SERVICE_ID, Execute::ID),
            "local",
            bytes.as_ref(),
        )
        .await?
        .as_slice(),
    )?)
}

fn main() -> failure::Fallible<()> {
    env_logger::init();
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
        Args::Ping { dst, msg } => {
            let result = sys.block_on(actix_rpc::service(&dst).send(Ping(msg)))?;
            eprintln!("got result: {:?}", result);
        }

        Args::Local { script } => {
            let _ = ExeUnit::default().start();

            let result = sys.block_on(async {
                tokio::time::delay_for(Duration::from_millis(500)).await;
                run_script(script).await
            })?;
            eprintln!("got result: {:?}", result);
        }
        Args::LocalRaw { script } => {
            let _ = ExeUnit::default().start();

            let result = sys.block_on(async {
                tokio::time::delay_for(Duration::from_millis(500)).await;
                run_script_raw(script).await.map_err(|e| format!("{}", e))?
            });
            eprintln!("got result: {:?}", result);
        }
    }
    Ok(())
}
