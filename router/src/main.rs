use std::clone::Clone;
use std::sync::{Arc, Mutex};

use structopt::StructOpt;
use tokio::codec::{FramedRead, FramedWrite};
use tokio::net::TcpListener;
use tokio::prelude::*;

use ya_sb_proto::codec::{GsbMessageDecoder, GsbMessageEncoder};
use ya_sb_router::Router;

#[derive(StructOpt)]
#[structopt(name = "Router", about = "Service Bus Router")]
struct Options {
    #[structopt(short = "l", default_value = "127.0.0.1:8245")]
    ip_port: String,
}

// TODO: move to example
fn main() {
    let options = Options::from_args();
    let listen_addr = options.ip_port.parse().expect("Invalid ip:port");
    let listener = TcpListener::bind(&listen_addr).expect("Unable to bind TCP listener");
    let router = Arc::new(Mutex::new(Router::new()));

    let server = listener
        .incoming()
        .map_err(|e| eprintln!("Accept failed: {:?}", e))
        .for_each(move |sock| {
            let addr = sock.peer_addr().unwrap();
            let (reader, writer) = sock.split();
            let writer = FramedWrite::new(writer, GsbMessageEncoder {});
            let reader = FramedRead::new(reader, GsbMessageDecoder::new());

            router
                .lock()
                .unwrap()
                .connect(addr.clone(), writer)
                .unwrap();
            let router1 = router.clone();
            let router2 = router.clone();

            tokio::spawn(
                reader
                    .from_err()
                    .for_each(move |msg| {
                        future::done(router1.lock().unwrap().handle_message(addr.clone(), msg))
                    })
                    .and_then(move |_| future::done(router2.lock().unwrap().disconnect(&addr)))
                    .map_err(|e| eprintln!("Error occurred handling message: {:?}", e)),
            )
        });

    tokio::run(server);
}
