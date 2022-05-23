use anyhow::anyhow;
use dotenv;
use env_logger;
use futures::future::{AbortHandle, Abortable};
use std::sync::{Arc, Mutex};
use structopt::StructOpt;
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};

use figment::Figment;

use rocket::response::content::RawHtml;
use rocket::serde::{Deserialize, Serialize};
use rocket::{get, routes};

struct TransportLoopData {
    _synched: i32,
    transferred: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
struct Config {
    app_value: usize,
    /* and so on.. */
}

impl Default for Config {
    fn default() -> Config {
        Config { app_value: 3 }
    }
}

async fn transport_loop(
    mut rd: ReadHalf<TcpStream>,
    mut wd: WriteHalf<TcpStream>,
    shared_data: Arc<Mutex<TransportLoopData>>,
) -> anyhow::Result<()> {
    let mut buf = vec![0; 16];

    loop {
        let n = match rd.read(&mut buf).await {
            Ok(n) => n,
            Err(err) => {
                log::error!("Error when reading {:?}", err);
                break;
            }
        };

        {
            let mut shared_data = shared_data.lock().unwrap();
            shared_data.transferred += n as u64;
        }

        if n == 0 {
            log::error!("Read 0 bytes");
            break;
        }
        println!("GOT {:?}", &buf[..n]);
        wd.write_all(&buf[..n]).await.unwrap();
    }
    Ok(())
}

async fn process_socket(socket: TcpStream, target: TcpStream) -> anyhow::Result<()> {
    // do work with socket here
    let (rd, wr) = tokio::io::split(socket);
    let (rdt, wrt) = tokio::io::split(target);
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    let (abort_handle2, abort_registration2) = AbortHandle::new_pair();

    let sent_data = Arc::new(Mutex::new(TransportLoopData {
        _synched: 1,
        transferred: 1,
    }));
    let receive_data = Arc::new(Mutex::new(TransportLoopData {
        _synched: 1,
        transferred: 1,
    }));

    tokio::spawn(async move {
        let reader_future = Abortable::new(
            async move {
                match transport_loop(rd, wrt, sent_data).await {
                    Ok(()) => {
                        log::info!("Transport loop finished");
                    }
                    Err(err) => {
                        log::error!("Transport loop finished with error: {:?}", err);
                    }
                }

                abort_handle2.abort();
            },
            abort_registration,
        );
        match reader_future.await {
            Ok(()) => {
                log::error!("Reader part of p9 communication ended too soon");
            }
            Err(e) => {
                log::info!("Future aborted, reason {e}");
            }
        }
    });
    let writer_future = Abortable::new(
        async move {
            match transport_loop(rdt, wr, receive_data).await {
                Ok(()) => {
                    log::info!("Transport loop finished");
                }
                Err(err) => {
                    log::error!("Transport loop finished with error: {:?}", err);
                }
            }

            /*
            let mut buf = vec![0; 16];


            loop {
                let n = match rdt.read(&mut buf).await {
                    Ok(n) => n,
                    Err(err) => {
                        log::error!("Error when reading {:?}", err);
                        break;
                    }
                };

                if n == 0 {
                    log::error!("Read 0 bytes");
                    break;
                }
                let s = String::from_utf8_lossy(&buf[..n]);
                println!("GOT from server {:?}", s);
                wr.write_all(&buf[..n]).await.unwrap();
            }*/
            abort_handle.abort();
        },
        abort_registration2,
    );

    match writer_future.await {
        Ok(()) => {
            log::error!("Reader part of p9 communication ended too soon");
        }
        Err(e) => {
            log::info!("Future aborted, reason {e}");
        }
    }

    Ok(())
}

#[derive(Debug, StructOpt)]
#[structopt(name = "options", about = "Options for data limiter relay")]
struct Opt {
    // we don't want to name it "speed", need to look smart
    #[structopt(long = "--listen-addr", default_value = "127.0.0.1:22444")]
    listen_addr: String,
    // we don't want to name it "speed", need to look smart
    #[structopt(long = "--target-addr", default_value = "127.0.0.1:17777")]
    target_addr: String,
    // we don't want to name it "speed", need to look smart
    #[structopt(long = "--rest-api-addr", default_value = "127.0.0.1:22000")]
    rest_api_addr: String,
}

#[get("/")] // <- route attribute
fn main_route() -> RawHtml<&'static str> {
    RawHtml(
        r#"<ul><li><a href="/world">Hello world</a></li><li><a href="/stats">Statistics</a></li></ul>"#,
    )
}

#[get("/world")] // <- route attribute
fn world() -> RawHtml<&'static str> {
    // <- request handler
    RawHtml(r#"<div style="border:1px solid gray">Hello world</div>"#)
}

#[get("/stats")] // <- route attribute
fn stats() -> RawHtml<&'static str> {
    // <- request handler
    RawHtml(r#"<div style="border:1px solid gray">Hello world</div>"#)
}

async fn run_rocket() -> anyhow::Result<()> {
    let opt = Opt::from_args();
    let str: String = opt.rest_api_addr;
    let split: Vec<&str> = str.split(":").collect();

    let address = split.get(0).ok_or(anyhow!("failed to split address"))?;
    let port_str = split.get(1).ok_or(anyhow!("failed to split address"))?;
    let port = port_str.parse::<u16>()?;

    log::debug!("Starting rocket at address {}:{}", address, port);

    let cfg = Figment::from(rocket::config::Config::default())
        .merge(("port", port))
        .merge(("address", address));
    let _rock = rocket::custom(cfg)
        .mount("/", routes![main_route, world, stats])
        .launch()
        .await?;
    Ok(())
}

async fn run() -> anyhow::Result<()> {
    let opt = Opt::from_args();
    dotenv::dotenv().ok();

    log::debug!("Run ...");

    log::debug!(
        "Starting relay. Listen addr: {}, target addr: {}",
        opt.listen_addr,
        opt.target_addr
    );

    tokio::spawn(async move {
        {
            match run_rocket().await {
                Ok(()) => {
                    log::info!("Rocket finished without error");
                }
                Err(err) => {
                    log::error!("Rocket ended with error: {}", err);
                }
            }
        }
    });
    let listener = TcpListener::bind(opt.listen_addr).await?;
    let (socket, _) = listener.accept().await?;
    let connection = TcpStream::connect(opt.target_addr).await?;

    process_socket(socket, connection).await?;

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();

    match run().await {
        Ok(()) => {
            log::info!("Relay ended successfully");
        }
        Err(err) => {
            log::error!("Relay ended with err {:?}", err);
        }
    }
}
