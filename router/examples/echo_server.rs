use futures::prelude::*;
use tokio::prelude::*;

use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::tcp_connect;

async fn run_server() {
    let router_addr = "127.0.0.1:8245".parse().unwrap();
    let (mut writer, mut reader) = tcp_connect(&router_addr).await;

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
        _ => panic!("Unexpected message received"),
    }

    reader
        .filter_map(|msg| {
            async {
                match msg {
                    Ok(GsbMessage::CallRequest(msg)) => {
                        println!(
                            "Received call request request_id = {} caller = {} address = {}",
                            msg.request_id, msg.caller, msg.address
                        );
                        Some(Ok(CallReply {
                            request_id: msg.request_id,
                            code: CallReplyCode::CallReplyOk as i32,
                            reply_type: CallReplyType::Full as i32,
                            data: msg.data,
                        }
                        .into()))
                    }
                    _ => {
                        eprintln!("Unexpected message received");
                        None
                    }
                }
            }
        })
        .forward(writer)
        .map(|_| ())
        .await;
}

#[tokio::main]
async fn main() {
    run_server().await;
}
