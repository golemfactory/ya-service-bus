use futures::prelude::*;
#[cfg(feature = "tls")]
use std::net::SocketAddr;

use uuid::Uuid;

use clap::Parser;
use std::time::Duration;
use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::*;

async fn run_client(args: Args) {
    #[cfg(feature = "tls")]
    let (mut writer, mut reader) = tls_connect(args.server, args.cert).await.unwrap();

    #[cfg(not(feature = "tls"))]
    let (mut writer, mut reader) = connect(GsbAddr::default()).await;

    let instance_id = Uuid::new_v4().as_bytes().to_vec();
    let payload = [0u8; 200000];
    println!("Sending hello...");
    let hello = Hello {
        name: "echo-client".to_string(),
        version: "0.0".to_string(),
        instance_id,
        ..Default::default()
    };
    writer
        .send(GsbMessage::Hello(hello))
        .await
        .expect("Send failed");
    let _msg = reader.next().await.unwrap().expect("Reply not received");

    println!("Sending call request...");

    let requests: Vec<CallRequest> = (0u64..args.count.unwrap_or(1))
        .map(|idx| {
            let request_id = format!("{}-{}", Uuid::new_v4(), idx);
            CallRequest {
                caller: "".to_string(),
                address: "echo/test".to_string(),
                request_id: request_id.clone(),
                data: payload.to_vec(),
                no_reply: false,
            }
        })
        .collect();

    let senders = async {
        for request in &requests {
            println!("sending {}", &request.request_id);
            writer
                .send(request.clone().into())
                .await
                .expect("Send failed");
            println!("sending done");
            if let Some(delay_secs) = &args.delay {
                tokio::time::sleep(Duration::from_secs(*delay_secs)).await;
            }
        }
    };

    let recv = async {
        for request in &requests {
            let msg = reader.next().await.unwrap().expect("Reply not received");
            match msg {
                GsbMessage::CallReply(msg) => {
                    println!("Call reply received {}", msg.request_id);
                    if msg.request_id != request.request_id {
                        println!(
                            "Wrong request_id: {} != {}",
                            msg.request_id, request.request_id
                        );
                    }

                    if msg.data != request.data {
                        println!("Wrong payload: {:?}", &msg.data[..20]);
                    }
                }
                _ => {
                    println!("Unexpected message received");
                }
            }
        }
    };

    let (_, _) = future::join(senders, recv).await;
}

#[derive(Parser)]
struct Args {
    #[arg(long, short)]
    delay: Option<u64>,
    #[arg(long, short)]
    count: Option<u64>,
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
async fn main() {
    rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
        .unwrap();

    run_client(Args::parse()).await;
}
