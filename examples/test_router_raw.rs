use actix::prelude::*;
use futures::prelude::*;

use futures::channel::oneshot;
use std::error::Error;
use std::{env, path::PathBuf, time::Duration};
use structopt::StructOpt;
use ya_service_bus::connection::{CallRequestHandler, ClientInfo};
use ya_service_bus::{connection, ResponseChunk};

const BAST_TOPIC: &str = "bcastecho";
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
struct DebugHandler(Option<oneshot::Sender<()>>);

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

    fn handle_event(&mut self, caller: String, topic: String, data: Vec<u8>) {
        println!(
            r#"
                      _    |
  _____   _____ _ __ | |_  | caller:     {}
 / _ \ \ / / _ \ '_ \| __| | topic:      {}
|  __/\ V /  __/ | | | |_  |
 \___| \_/ \___|_| |_|\__| |
--
{}
--
        "#,
            caller,
            topic,
            String::from_utf8_lossy(data.as_ref())
        );
    }

    fn on_disconnect(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    env::set_var("RUST_LOG", env::var("RUST_LOG").unwrap_or("debug".into()));
    env_logger::init();
    let bus_addr = "127.0.0.1:7464";
    let args = Args::from_args();
    let mut sys = System::new("");
    sys.block_on(async move {
        let mut client_info = ClientInfo::default();
        let (tx, rx) = oneshot::channel();
        client_info.name = "test_router_raw".to_string();

        let connection = connection::connect_with_handler(
            client_info,
            connection::tcp(bus_addr).await?,
            DebugHandler(Some(tx)),
        );
        match args {
            Args::EventListener { time } => {
                connection.subscribe(BAST_TOPIC).await?;
                delay_for(time).await;
                connection.unsubscribe(BAST_TOPIC).await?;
                Ok(())
            }
            Args::Server {
                subscribe, time, ..
            } => {
                connection.bind(SERVICE_ADDR).await.expect("bind echo");
                if subscribe {
                    connection.subscribe(BAST_TOPIC).await?;
                }

                let w = delay_for(time).fuse();
                futures::pin_mut!(w, rx);
                future::select(w, rx).await;

                if subscribe {
                    connection.unsubscribe(BAST_TOPIC).await?;
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
                connection.broadcast("maj_ajdi", BAST_TOPIC, data).await?;
                Ok(())
            }
        }
    })
}
