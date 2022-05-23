use anyhow::anyhow;
use chrono::{DateTime, Utc};
use dotenv;
use env_logger;
use futures::future::{AbortHandle, Abortable};
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use structopt::StructOpt;
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};

use figment::Figment;
use lazy_static::lazy_static;

use rand::rngs::StdRng;
use rand::Rng;
use rand::SeedableRng;
use rocket::response::content::{RawHtml, RawJson};
use rocket::{get, routes};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::time::Duration;

#[derive(Default, Serialize, Deserialize, Clone)]
struct PacketInfo {
    no: u64,
    time: Option<DateTime<Utc>>,
    size: u64,
    delay_ms: u64,
    data_hex: String,
}

#[derive(Default, Serialize, Deserialize, Clone)]
struct TransportLoopData {
    random_seed: u64,
    started: i32,
    transferred: u64,
    packets: Vec<PacketInfo>,
}
lazy_static! {
    /// This is an example for using doc comment attributes
    static ref SENT_DATA_STATIC: Arc<Mutex<TransportLoopData>> = Arc::new(Mutex::new(TransportLoopData::default()));
    static ref RECEIVE_DATA_STATIC: Arc<Mutex<TransportLoopData>> = Arc::new(Mutex::new(TransportLoopData::default()));
}

async fn transport_loop(
    mut rd: ReadHalf<TcpStream>,
    mut wd: WriteHalf<TcpStream>,
    shared_data: Arc<Mutex<TransportLoopData>>,
) -> anyhow::Result<()> {
    {
        let mut shared_data = shared_data.lock().unwrap();
        shared_data.started = 1;
    }
    let mut rng = {
        let mut shared_data = shared_data.lock().unwrap();
        shared_data.random_seed =
            chrono::Utc::now().timestamp_millis() as u64 + shared_data.random_seed;

        StdRng::seed_from_u64(shared_data.random_seed)
    };

    let mut packet_no = 0u64;
    loop {
        let rand1 = rng.gen_range(1..100);

        let buf_size = rand1;
        let mut buf = vec![0; buf_size];
        let n = match rd.read(&mut buf).await {
            Ok(n) => n,
            Err(err) => {
                log::error!("Error when reading {:?}", err);
                break;
            }
        };

        let delay_ms = rng.gen_range(1..1000);
        {
            let mut shared_data = shared_data.lock().unwrap();
            shared_data.transferred += n as u64;
            let mut s = String::with_capacity(n * 2);
            for &byte in &buf[..n] {
                write!(&mut s, "{:02x}", byte).expect("Unable to write");
            }
            let packet = PacketInfo {
                time: Some(Utc::now()),
                no: packet_no,
                delay_ms: delay_ms,
                data_hex: s,
                size: n as u64,
            };
            shared_data.packets.push(packet);
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;

        if n == 0 {
            log::error!("Read 0 bytes");
            break;
        }
        println!("GOT {:?}", &buf[..n]);
        wd.write_all(&buf[..n]).await.unwrap();
        packet_no += 1;
    }
    Ok(())
}

async fn process_socket(socket: TcpStream, target: TcpStream) -> anyhow::Result<()> {
    // do work with socket here
    let (rd, wr) = tokio::io::split(socket);
    let (rdt, wrt) = tokio::io::split(target);
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    let (abort_handle2, abort_registration2) = AbortHandle::new_pair();

    let sent_data = (*SENT_DATA_STATIC).clone();
    let receive_data = (*RECEIVE_DATA_STATIC).clone();

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
fn stats() -> RawJson<String> {
    // <- request handler
    let str = {
        let sent_data = (*SENT_DATA_STATIC).lock().unwrap();
        serde_json::to_string(sent_data.deref())
    };

    let str = str.unwrap();
    RawJson(str)
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

    let (abort_rocket_handle, abort_rocket_registration) = AbortHandle::new_pair();

    tokio::spawn(async move {
        let rocket_future = Abortable::new(
            async move {
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
            },
            abort_rocket_registration,
        );

        match rocket_future.await {
            Ok(()) => {
                log::error!("Rocket finished with error");
            }
            Err(e) => {
                log::info!("Rocket aborted, reason {e}");
            }
        }
    });
    let listener = TcpListener::bind(opt.listen_addr).await?;
    let (socket, _) = listener.accept().await?;
    let connection = TcpStream::connect(opt.target_addr).await?;

    process_socket(socket, connection).await?;
    abort_rocket_handle.abort();

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
