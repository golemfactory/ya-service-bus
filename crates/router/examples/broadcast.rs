use std::convert::TryInto;
use std::time::Duration;

use futures::prelude::*;
use structopt::*;
use ubyte::*;
use uuid::Uuid;

use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::connect;

async fn run_client(args: Args) -> anyhow::Result<()> {
    let (mut writer, mut reader) = connect(Default::default()).await;

    let instance_id = Uuid::new_v4().as_bytes().to_vec();
    println!("Sending hello...");
    let hello = Hello {
        name: "broadcast-client".to_string(),
        version: "0.0".to_string(),
        instance_id,
        ..Default::default()
    };
    writer
        .send(GsbMessage::Hello(hello))
        .await
        .expect("Send failed");
    let _msg = reader.next().await.unwrap().expect("Reply not received");

    println!("Sending subscribe request...");
    let topic = "test";
    let subscribe_request = SubscribeRequest {
        topic: topic.to_string(),
    };
    writer
        .send(subscribe_request.into())
        .await
        .expect("Send failed");

    let msg = reader.next().await.unwrap().expect("Reply not received");
    match msg {
        GsbMessage::SubscribeReply(msg) => {
            println!("Subscribe reply received");
            assert!(
                msg.code == SubscribeReplyCode::SubscribedOk as i32,
                "Non-zero reply code"
            )
        }
        GsbMessage::Ping(_) => {}
        _ => panic!("Unexpected message received"),
    }

    for _ in 0..args.count.unwrap_or(1) {
        if let Some(delay) = args.delay {
            println!("pause for {} secs", delay);
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
        println!("Sending broadcast request...");
        let mut broadcast_data = Vec::new();
        broadcast_data.resize(args.payload_size.as_u64().try_into()?, 0u8);

        let broadcast_request = BroadcastRequest {
            caller: "some_id".into(),
            topic: topic.to_string(),
            data: broadcast_data,
        };
        writer
            .send(broadcast_request.clone().into())
            .await
            .expect("Send failed");

        loop {
            let msg = reader.next().await.unwrap().expect("Reply not received");
            match msg {
                GsbMessage::BroadcastRequest(_msg) => {
                    println!("Broadcast message received");
                }
                GsbMessage::BroadcastReply(msg) => {
                    println!("Broadcast reply received");
                    assert!(
                        msg.code == BroadcastReplyCode::BroadcastOk as i32,
                        "Non-zero reply code"
                    );
                    break;
                }
                GsbMessage::Ping(_) => {}
                _ => panic!("Unexpected message received"),
            }
        }

        loop {
            let msg = reader
                .next()
                .await
                .unwrap()
                .expect("Broadcast message not received");
            match msg {
                GsbMessage::BroadcastRequest(msg) => {
                    println!("Broadcast message received");
                    if msg == broadcast_request {
                        break;
                    }
                }
                GsbMessage::Ping(_) => {}
                _ => panic!("Unexpected message received"),
            }
        }
    }

    println!("Sending unsubscribe request...");
    let unsubscribe_request = UnsubscribeRequest {
        topic: topic.to_string(),
    };
    writer
        .send(unsubscribe_request.into())
        .await
        .expect("Send failed");

    loop {
        let msg = reader.next().await.unwrap().expect("Reply not received");
        match msg {
            GsbMessage::UnsubscribeReply(msg) => {
                println!("Unsubscribe reply received");
                assert!(
                    msg.code == UnsubscribeReplyCode::UnsubscribedOk as i32,
                    "Non-zero reply code"
                );
                break;
            }
            GsbMessage::BroadcastRequest(_) => {
                println!("Broadcast message received");
            }
            GsbMessage::Ping(_) => {}
            msg => panic!("Unexpected message received: {:?}", msg),
        }
    }
    Ok(())
}

#[derive(StructOpt, Clone)]
struct Args {
    #[structopt(long, short)]
    delay: Option<u64>,
    #[structopt(long, short)]
    count: Option<usize>,
    #[structopt(long, default_value = "10kb")]
    payload_size: ByteUnit,
    #[structopt(long, short)]
    parallel: Option<usize>,
}

#[actix_rt::main]
async fn main() {
    env_logger::init();
    let args = Args::from_args();
    if let Some(n) = args.parallel {
        let handles = (0..n).map(|_| run_client(args.clone()));

        let _ = future::join_all(handles).await;
    } else {
        run_client(Args::from_args()).await.unwrap();
    }
}
