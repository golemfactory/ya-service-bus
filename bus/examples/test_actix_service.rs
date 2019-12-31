use actix::prelude::*;
use failure::_core::time::Duration;
use futures::{FutureExt, StreamExt, TryFutureExt};
use futures_01::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::path::PathBuf;
use structopt::StructOpt;
use ya_service_bus::{
    actix_rpc, untyped, Error, Handle, RpcEnvelope, RpcMessage, RpcStreamCall, RpcStreamMessage,
};

#[derive(Serialize, Deserialize)]
struct Ping(String);

impl RpcMessage for Ping {
    const ID: &'static str = "ping";
    type Item = String;
    type Error = ();
}

impl RpcStreamMessage for Ping {
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

impl RpcStreamMessage for Execute {
    const ID: &'static str = "execute";
    type Item = String;
    type Error = String;
}

#[derive(Default)]
struct ExeUnit(Option<Handle>);

impl Actor for ExeUnit {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.0 = Some(actix_rpc::binds::<Execute>(
            SERVICE_ID,
            ctx.address().recipient(),
        ))
    }
}

impl Handler<RpcStreamCall<Execute>> for ExeUnit {
    type Result = ActorResponse<Self, (), Error>;

    fn handle(&mut self, msg: RpcStreamCall<Execute>, _ctx: &mut Self::Context) -> Self::Result {
        eprintln!("got {:?}", msg.body);
        let mut it = msg.body.0.into_iter();
        let s = futures_01::stream::poll_fn(move || -> Result<_, Error> {
            if let Some(command) = it.next() {
                Ok(Async::Ready(Some(Ok(format!("{:?}", command)))))
            } else {
                Ok(Async::Ready(None))
            }
        });

        ActorResponse::r#async(
            msg.reply
                .sink_map_err(|s| Error::Closed)
                .send_all(s)
                .and_then(|_| Ok::<_, Error>(()))
                .into_actor(self),
        )
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
    Pings {
        dst: String,
        msg: String,
    },
}

fn run_script(script: PathBuf) -> impl Future<Item = Vec<String>, Error = failure::Error> {
    (|| -> Result<_, std::io::Error> {
        let commands: Vec<Command> =
            serde_json::from_reader(OpenOptions::new().read(true).open(script)?)?;
        Ok(commands)
    })()
    .into_future()
    .from_err()
    .and_then(|commands| {
        actix_rpc::service(SERVICE_ID)
            .call_stream(Execute(commands))
            .from_err()
            .collect()
            .and_then(|v| {
                let mut results = Vec::new();

                for it in v {
                    results.push(it.map_err(|e| failure::err_msg(e))?)
                }
                Ok(results)
            })
    })
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

        Args::Pings { dst, msg } => {
            let result = sys.block_on(
                actix_rpc::service(&dst)
                    .call_stream(Ping(msg))
                    .for_each(|result| Ok(eprintln!("got result: {:?}", result))),
            )?;
            eprintln!("done = {:?}", result);
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
