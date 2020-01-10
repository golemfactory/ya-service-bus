use futures::prelude::*;

use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::tcp_connect;

async fn run_client() {
    let router_addr = "127.0.0.1:8245".parse().unwrap();
    let (mut writer, mut reader) = tcp_connect(&router_addr).await;

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
        _ => panic!("Unexpected message received"),
    }

    println!("Sending broadcast request...");
    let broadcast_data = "broadcast";
    let broadcast_request = BroadcastRequest {
        topic: topic.to_string(),
        data: broadcast_data.to_string().into_bytes(),
    };
    writer
        .send(broadcast_request.clone().into())
        .await
        .expect("Send failed");

    let msg = reader.next().await.unwrap().expect("Reply not received");
    match msg {
        GsbMessage::BroadcastReply(msg) => {
            println!("Broadcast reply received");
            assert!(
                msg.code == BroadcastReplyCode::BroadcastOk as i32,
                "Non-zero reply code"
            )
        }
        _ => panic!("Unexpected message received"),
    }

    let msg = reader
        .next()
        .await
        .unwrap()
        .expect("Broadcast message not received");
    match msg {
        GsbMessage::BroadcastRequest(msg) => {
            println!("Broadcast message received");
            assert!(msg == broadcast_request, "Wrong data received")
        }
        _ => panic!("Unexpected message received"),
    }

    println!("Sending unsubscribe request...");
    let unsubscribe_request = UnsubscribeRequest {
        topic: topic.to_string(),
    };
    writer
        .send(unsubscribe_request.into())
        .await
        .expect("Send failed");

    let msg = reader.next().await.unwrap().expect("Reply not received");
    match msg {
        GsbMessage::UnsubscribeReply(msg) => {
            println!("Unsubscribe reply received");
            assert!(
                msg.code == UnsubscribeReplyCode::UnsubscribedOk as i32,
                "Non-zero reply code"
            )
        }
        _ => panic!("Unexpected message received"),
    }
}

#[tokio::main]
async fn main() {
    run_client().await;
}
