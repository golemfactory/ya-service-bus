use actix::prelude::*;
use futures::prelude::*;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::time::Duration;
use std::{env, fs::OpenOptions, path::PathBuf};
use structopt::StructOpt;
use ya_service_bus::{actix_rpc, Handle, RpcEnvelope, RpcMessage, RpcStreamMessage};

#[derive(Serialize, Deserialize)]
struct Ping(String);

impl RpcMessage for Ping {
    const ID: &'static str = "ping";
    type Item = String;
    type Error = ();
}

#[derive(Serialize, Deserialize)]
struct StreamPing(String);

impl RpcStreamMessage for StreamPing {
    const ID: &'static str = "ping";
    type Item = String;
    type Error = ();
}

const BUS_ID: &str = "/local/exeunit";

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
            BUS_ID,
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
    Ping {
        dst: String,
        msg: String,
    },
    StreamPing {
        dst: String,
        msg: String,
    },
}

fn run_script(script: PathBuf) -> impl Future<Output = Result<String, Box<dyn Error>>> {
    async move {
        let commands: Vec<Command> =
            serde_json::from_reader(OpenOptions::new().read(true).open(script)?)?;
        let result = actix_rpc::service(BUS_ID).send(Execute(commands)).await?;
        result.map_err(From::from)
    }
}

#[actix_rt::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env::set_var("RUST_LOG", env::var("RUST_LOG").unwrap_or("debug".into()));
    env_logger::init();
    let args = Args::from_args();
    match args {
        Args::Server { .. } => {
            let _ = ExeUnit::default().start();
            tokio::signal::ctrl_c().await?;
            eprintln!("done");
        }
        Args::Client { script } => {
            let result = run_script(script).await?;
            eprintln!("got result: {:?}", result);
        }
        Args::Ping { dst, msg } => {
            let result = actix_rpc::service(&dst).send(Ping(msg)).await?;
            eprintln!("got result: {:?}", result);
        }
        Args::StreamPing { dst, msg } => {
            let result = actix_rpc::service(&dst)
                .call_stream(StreamPing(msg))
                .for_each(|item| future::ready(eprintln!("got={:?}", item)))
                .await;
            eprintln!("got result: {:?}", result);
        }

        Args::Local { script } => {
            let _ = ExeUnit::default().start();

            let result = async {
                actix::clock::sleep(Duration::from_millis(500)).await;
                run_script(script).await
            }
            .await?;
            eprintln!("got result: {:?}", result);
        }
    }
    Ok(())
}
