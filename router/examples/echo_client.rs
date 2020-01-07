use futures::prelude::*;
use tokio::prelude::*;
use uuid::Uuid;

use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::tcp_connect;

async fn run_client() {
    let router_addr = "127.0.0.1:8245".parse().unwrap();
    let (mut writer, mut reader) = tcp_connect(&router_addr).await;

    println!("Sending call request...");
    let request_id = Uuid::new_v4().to_hyphenated().to_string();
    let hello_msg = "Hello";
    let call_request = CallRequest {
        caller: "".to_string(),
        address: "echo/test".to_string(),
        request_id: request_id.clone(),
        data: hello_msg.to_string().into_bytes(),
    };
    writer.send(call_request.into()).await.expect("Send failed");

    let msg = reader.next().await.unwrap().expect("Reply not received");
    match msg {
        GsbMessage::CallReply(msg) => {
            println!("Call reply received");
            if msg.request_id != request_id {
                println!("Wrong request_id: {} != {}", msg.request_id, request_id);
            }
            let recv_msg = String::from_utf8(msg.data).expect("Not a valid UTF-8 string");
            if recv_msg != hello_msg {
                println!("Wrong payload: {} != {}", recv_msg, hello_msg);
            }
        }
        _ => {
            println!("Unexpected message received");
        }
    }
}

#[tokio::main]
async fn main() {
    run_client().await;
}
