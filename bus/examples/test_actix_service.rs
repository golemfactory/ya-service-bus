use actix::prelude::*;
use failure::_core::time::Duration;
use futures::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::path::PathBuf;
use structopt::StructOpt;
use ya_service_bus::{
    actix_rpc, untyped, Error, Handle, RpcMessage, RpcStreamCall, RpcStreamMessage,
};
use futures::SinkExt;
use futures::task::Poll;

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
        let reply = msg.reply;
        let mut s = stream::poll_fn(move |_| -> Poll<Option<Result<Result<String, String>, Error>>>{
            if let Some(command) = it.next() {
                Poll::Ready(Some(Ok(Ok(format!("{:?}", command)))))
            } else {
                Poll::Ready(None)
            }
        });

        ActorResponse::r#async(async move {
            let v = reply.sink_map_err(|_| Error::Closed).
                send_all(&mut s).await;
            eprintln!("r={:?}", v);
            Ok(())
        }.into_actor(self))
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

fn run_script(script: PathBuf) -> impl Future<Output = Result<Vec<String>, failure::Error>> {
    async move {
        let commands: Vec<Command> =
            serde_json::from_reader(OpenOptions::new().read(true).open(script)?)?;
        let result: Result<Vec<_>, _> = actix_rpc::service(SERVICE_ID)
            .call_stream(Execute(commands))
            .try_collect()
            .await;

        let it = result?;

        it.into_iter().collect::<Result<Vec<_>,_>>().map_err(|_| failure::err_msg("invalid"))

        //result.)
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

        Args::Pings { dst, msg } => {
            let result = sys.block_on(
                actix_rpc::service(&dst)
                    .call_stream(Ping(msg))
                    .try_for_each(|result| future::ok(eprintln!("got result: {:?}", result))),
            )?;
            eprintln!("done = {:?}", result);
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
