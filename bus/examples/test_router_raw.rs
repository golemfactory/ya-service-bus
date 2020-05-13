use actix::prelude::*;
use futures::prelude::*;

use std::error::Error;
use std::{env, path::PathBuf, time::Duration};
use structopt::StructOpt;
use ya_service_bus::connection::CallRequestHandler;
use ya_service_bus::{connection, ResponseChunk};

const BAST_ADDR: &str = "bcastecho";
const SERVICE_ADDR: &str = "/local/raw/echo";

async fn delay_for(secs: Option<u64>) {
    if let Some(secs) = secs {
        tokio::time::delay_for(Duration::from_secs(secs)).await
    } else {
        future::pending().await
    }
}

#[derive(StructOpt)]
enum Args {
    /// Starts server that waits for commands on gsb://local/exe-unit
    Server {
        #[structopt(short)]
        subscribe: bool,
        #[structopt(short)]
        time: Option<u64>,
    },
    /// Sends script to gsb://local/exe-unit service
    Send {
        script: PathBuf,
    },

    Broadcast {
        script: PathBuf,
    },
    EventListener {
        #[structopt(short)]
        time: Option<u64>,
    },
}

#[derive(Default)]
struct DebugHandler;

impl CallRequestHandler for DebugHandler {
    type Reply = stream::Once<future::Ready<Result<ResponseChunk, ya_service_bus::Error>>>;

    fn do_call(
        &mut self,
        request_id: String,
        caller: String,
        address: String,
        data: Vec<u8>,
    ) -> Self::Reply {
        println!(
            r#"
           _                |
  ___  ___| |__   ___       | address:    {}
 / _ \/ __| '_ \ / _ \      | request id: {}
|  __/ (__| | | | (_) |     | caller:     {}
 \___|\___|_| |_|\___/      |
--
{}
--
        "#,
            address,
            request_id,
            caller,
            String::from_utf8_lossy(data.as_ref())
        );

        stream::once(future::ok(ResponseChunk::Full(data)))
    }

    fn handle_event(&mut self, address: String, data: Vec<u8>) {
        println!(
            r#"
                      _    |
  _____   _____ _ __ | |_  |
 / _ \ \ / / _ \ '_ \| __| | address:    {}
|  __/\ V /  __/ | | | |_  |
 \___| \_/ \___|_| |_|\__| |
--
{}
--
        "#,
            address,
            String::from_utf8_lossy(data.as_ref())
        );
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    env::set_var("RUST_LOG", env::var("RUST_LOG").unwrap_or("debug".into()));
    env_logger::init();
    let bus_addr = "127.0.0.1:7464".parse().unwrap();
    let args = Args::from_args();
    let mut sys = System::new("");
    sys.block_on(async move {
        let connection = connection::connect::<_, DebugHandler>(connection::tcp(bus_addr).await?);
        match args {
            Args::EventListener { time } => {
                connection.subscribe(BAST_ADDR).await?;
                delay_for(time).await;
                connection.unsubscribe(BAST_ADDR).await?;
                Ok(())
            }
            Args::Server {
                subscribe, time, ..
            } => {
                connection.bind(SERVICE_ADDR).await.expect("bind echo");
                if subscribe {
                    connection.subscribe(BAST_ADDR).await?;
                }
                delay_for(time).await;
                if subscribe {
                    connection.unsubscribe(BAST_ADDR).await?;
                }
                connection.unbind(SERVICE_ADDR).await?;

                Ok(())
            }
            Args::Send { script } => {
                let data = std::fs::read(script)?;
                let msg = connection.call("me", SERVICE_ADDR, data).await?;
                eprintln!("body={}", String::from_utf8_lossy(msg.as_ref()));
                Ok(())
            }
            Args::Broadcast { script } => {
                let data = std::fs::read(script)?;
                connection.broadcast(BAST_ADDR, data).await?;
                Ok(())
            }
        }
    })
}
