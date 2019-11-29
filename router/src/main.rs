use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::codec::FramedRead;
use tokio::net::TcpListener;
use tokio::prelude::*;

use ya_sb_router::decoder::GsbMessageDecoder;

fn main() {
    let addr = "127.0.0.1:8080".parse().unwrap();
    let listener = TcpListener::bind(&addr).expect("unable to bind TCP listener");
    let writers = Arc::new(Mutex::new(HashMap::new()));

    let server = listener
        .incoming()
        .map_err(|e| eprintln!("accept failed = {:?}", e))
        .for_each(move |sock| {
            let (reader, writer) = sock.split();
            writers.lock().unwrap().insert(addr, writer);
            let reader = FramedRead::new(reader, GsbMessageDecoder::new());
            tokio::spawn(
                reader
                    .map_err(|e| eprintln!("read failed = {:?}", e))
                    .for_each(|msg| {
                        Ok(()) // TODO: Handle masseges
                    }),
            )
        });

    tokio::run(server);
}
