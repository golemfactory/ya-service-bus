use actix::prelude::*;
use failure::Fallible;
use futures_01::prelude::*;
use std::path::PathBuf;
use structopt::StructOpt;
use ya_service_bus::connection;

#[derive(StructOpt)]
enum Args {
    /// Starts server that waits for commands on gsb://local/exe-unit
    Server {},
    /// Sends script to gsb://local/exe-unit service
    Client { script: PathBuf },
}

fn main() -> Fallible<()> {
    let bus_addr = "127.0.0.1:8245".parse().unwrap();
    let args = Args::from_args();
    match args {
        Args::Server { .. } => {
            System::run(move || {
                let a = connection::tcp(&bus_addr).and_then(|tcp_connection| {
                    let c = connection::connect(tcp_connection);

                    let handle_echo = |caller: &str, addr: &str, msg: &[u8]| {
                        eprintln!("got msg from {} to {}", caller, addr);
                        eprintln!("body={}", String::from_utf8_lossy(msg));
                        let msg: Vec<u8> = msg.into();
                        async move { Ok(msg.into()) }
                    };

                    let _ = ya_service_bus::untyped::subscribe("/local/raw/echo", handle_echo);
                    Arbiter::spawn(
                        c.bind("/local/raw/echo")
                            .map_err(|e| eprintln!("err={}", e)),
                    );

                    Ok(())
                });
                Arbiter::spawn(a.map_err(|e| eprintln!("connect error={}", e)))
            })
            .unwrap();
        }
        Args::Client { script } => {
            let data = std::fs::read(script).unwrap();
            let a = connection::tcp(&bus_addr)
                .map_err(|e| eprintln!("connect error={}", e))
                .and_then(|tcp_connection| {
                    let c = connection::connect(tcp_connection);

                    c.call("me", "/local/raw/echo", data)
                        .and_then(|msg| {
                            eprintln!("body={}", String::from_utf8_lossy(msg.as_ref()));
                            Ok(())
                        })
                        .map_err(|e| eprintln!("send error={}", e))
                });
            Arbiter::spawn(a)
        }
    }
    Ok(())
}
