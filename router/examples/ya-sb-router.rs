use std::clone::Clone;
use std::sync::{Arc, Mutex};

use futures::prelude::*;
use structopt::StructOpt;
use tokio::net::TcpListener;
use tokio::prelude::*;
use tokio_util::codec::{FramedRead, FramedWrite};

use ya_sb_proto::codec::{GsbMessageCodec, GsbMessageDecoder, GsbMessageEncoder};
use ya_sb_router::Router;

#[derive(StructOpt)]
#[structopt(name = "Router", about = "Service Bus Router")]
struct Options {
    #[structopt(short = "l", default_value = "127.0.0.1:8245")]
    ip_port: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    flexi_logger::Logger::with_env_or_str("info,ya_sb_router=debug")
        .start()
        .unwrap();

    let options = Options::from_args();
    let listen_addr: std::net::SocketAddr = options.ip_port.parse().expect("Invalid ip:port");
    let mut listener = TcpListener::bind(&listen_addr)
        .await
        .expect("Unable to bind TCP listener");
    log::info!("listening on {:?}", listen_addr);
    let router = Arc::new(Mutex::new(Router::new()));

    let _ = listener
        .incoming()
        .map_err(|e| log::error!("Accept failed: {:?}", e))
        .try_for_each(move |mut sock| {
            let addr = sock.peer_addr().unwrap();
            let (writer, reader) =
                tokio_util::codec::Framed::new(sock, GsbMessageCodec::default()).split();

            router
                .lock()
                .unwrap()
                .connect(addr.clone(), writer)
                .unwrap();
            let router1 = router.clone();
            let router2 = router.clone();

            tokio::spawn(async move {
                let _ = reader
                    .for_each(move |msg| {
                        router1
                            .lock()
                            .unwrap()
                            .handle_message(addr.clone(), msg.unwrap());
                        future::ready(())
                    })
                    .await;
            });
            future::ok(())
        })
        .await;

    Ok(())
}
