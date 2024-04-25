#![allow(unused_imports)]
use futures::prelude::*;
use std::net::SocketAddr;

use anyhow::{bail, Context};
use clap::{arg, Parser};
use std::time::Duration;
use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::*;

async fn run_server(args: Args) -> anyhow::Result<()> {
    #[cfg(feature = "tls")]
    let (mut writer, mut reader) = tls_connect(args.server, args.cert)
        .await
        .context("connect to router")?;

    #[cfg(not(feature = "tls"))]
    let (mut writer, mut reader) = connect(GsbAddr::default()).await;

    println!("Sending hello");
    let hello = Hello {
        name: "echo-server".to_string(),
        version: "0.0".to_string(),
        instance_id: vec![1, 2, 3, 4],
    };
    writer
        .send(GsbMessage::Hello(hello))
        .await
        .expect("Send failed");

    if let GsbMessage::Hello(h) = reader.next().await.unwrap().expect("hello not received") {
        println!("got hello: {:?}", h);
    } else {
        bail!("Unexpected message received")
    }

    println!("Sending register request...");
    let register_request = RegisterRequest {
        service_id: "echo".to_string(),
    };
    writer
        .send(register_request.into())
        .await
        .expect("Send failed");

    let msg = reader
        .next()
        .await
        .unwrap()
        .expect("Register reply not received");
    match msg {
        GsbMessage::RegisterReply(msg) if msg.code == RegisterReplyCode::RegisteredOk as i32 => {
            println!("Service successfully registered")
        }
        GsbMessage::Ping(_) => {}
        _ => bail!("Unexpected message received"),
    }

    println!("Sending register request...");
    let register_request = RegisterRequest {
        service_id: "echo2".to_string(),
    };
    writer
        .send(register_request.into())
        .await
        .expect("Send failed");

    let msg = reader
        .next()
        .await
        .unwrap()
        .expect("Register reply not received");
    match msg {
        GsbMessage::RegisterReply(msg) if msg.code == RegisterReplyCode::RegisteredOk as i32 => {
            println!("Service successfully registered")
        }
        GsbMessage::Ping(_) => {}
        _ => panic!("Unexpected message received"),
    }

    reader
        .filter_map(|msg| async {
            match msg {
                Ok(GsbMessage::CallRequest(msg)) => {
                    println!(
                        "Received call request request_id = {} caller = {} address = {}",
                        msg.request_id, msg.caller, msg.address
                    );
                    if let Some(delay_secs) = args.delay.as_ref() {
                        println!("delay for {} secs", delay_secs);
                        tokio::time::sleep(Duration::from_secs(*delay_secs)).await;
                        println!("done");
                    }
                    Some(Ok(CallReply {
                        request_id: msg.request_id,
                        code: CallReplyCode::CallReplyOk as i32,
                        reply_type: CallReplyType::Full as i32,
                        data: msg.data,
                    }
                    .into()))
                }
                Ok(GsbMessage::Ping(_)) => {
                    println!("Ping received");
                    Some(Ok(GsbMessage::pong()))
                }
                _ => {
                    eprintln!("Unexpected message received");
                    None
                }
            }
        })
        .forward(writer)
        .map(|_| ())
        .await;
    Ok(())
}

#[derive(Parser)]
struct Args {
    #[arg(long, short)]
    delay: Option<u64>,
    #[cfg(feature = "tls")]
    #[arg(long, default_value = "18.185.178.4:7464")]
    server: SocketAddr,
    #[cfg(feature = "tls")]
    #[arg(
        long,
        default_value = "393479950594e7c676ba121033a677a1316f722460827e217c82d2b3"
    )]
    cert: CertHash,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    run_server(Args::parse()).await
}
