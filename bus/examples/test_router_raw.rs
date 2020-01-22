use actix::prelude::*;
use futures::prelude::*;

use std::error::Error;
use std::{env, path::PathBuf, time::Duration};
use structopt::StructOpt;
use ya_service_bus::connection;
use ya_service_bus::connection::LocalRouterHandler;

#[derive(StructOpt)]
enum Args {
    /// Starts server that waits for commands on gsb://local/exe-unit
    Server {},
    /// Sends script to gsb://local/exe-unit service
    Client { script: PathBuf },
}

fn main() -> Result<(), Box<dyn Error>> {
    env::set_var("RUST_LOG", env::var("RUST_LOG").unwrap_or("debug".into()));
    env_logger::init();
    let bus_addr = *ya_service_api::constants::YAGNA_BUS_ADDR;
    let args = Args::from_args();
    match args {
        Args::Server { .. } => {
            System::run(move || {
                let a = connection::tcp(bus_addr).and_then(|tcp_connection| {
                    async move {
                        let c = connection::connect::<_, LocalRouterHandler>(tcp_connection);

                        let handle_echo = |caller: &str, addr: &str, msg: &[u8]| {
                            eprintln!("got msg from {} to {}", caller, addr);
                            eprintln!("body={}", String::from_utf8_lossy(msg));
                            let msg: Vec<u8> = msg.into();
                            async move {
                                tokio::time::delay_for(Duration::from_secs(20)).await;
                                Ok(msg.into())
                            }
                        };

                        let _ = ya_service_bus::untyped::subscribe("/local/raw/echo", handle_echo);
                        Arbiter::spawn(async move {
                            c.bind("/local/raw/echo")
                                .await
                                .expect("unabled to bind echo")
                        });

                        Ok(())
                    }
                });
                Arbiter::spawn(async {
                    let _result = a.await;
                })
            })
            .unwrap();
        }
        Args::Client { script } => {
            let data = std::fs::read(script).unwrap();
            System::run(move || {
                let a = connection::tcp(bus_addr)
                    .map_err(From::from)
                    .and_then(|tcp_connection| {
                        async move {
                            let c = connection::connect::<_, LocalRouterHandler>(tcp_connection);

                            let msg = c.call("me", "/local/raw/echo", data).await?;
                            eprintln!("body={}", String::from_utf8_lossy(msg.as_ref()));
                            Ok::<_, Box<dyn Error>>(())
                        }
                    })
                    .then(|v| async move { v.unwrap_or_else(|e| eprintln!("send error={}", e)) });
                Arbiter::spawn(a)
            })
            .unwrap()
        }
    }
    Ok(())
}
