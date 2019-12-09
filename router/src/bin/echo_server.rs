use futures::compat::{Future01CompatExt, Stream01CompatExt};
use futures::{FutureExt, StreamExt, TryFutureExt, TryStreamExt};

use tokio::prelude::*;

use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::connect;

async fn run_server() {
    let router_addr = "127.0.0.1:8080".parse().unwrap();
    let (reader, writer) = connect(&router_addr).await;
    let mut reader = reader.compat();

    println!("Sending register request...");
    let register_request = RegisterRequest {
        service_id: "echo".to_string(),
    };
    let writer = writer
        .send(register_request.into())
        .compat()
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
        .compat()
        .filter_map(|msg| match msg {
            GsbMessage::CallRequest(msg) => {
                println!(
                    "Received call request request_id = {} caller = {} address = {}",
                    msg.request_id, msg.caller, msg.address
                );
                Some(
                    CallReply {
                        request_id: msg.request_id,
                        code: CallReplyCode::CallReplyOk as i32,
                        reply_type: CallReplyType::Full as i32,
                        data: msg.data,
                    }
                    .into(),
                )
            }
            _ => {
                eprintln!("Unexpected message received");
                None
            }
        })
        .forward(writer)
        .compat()
        .map(|_| ())
        .await;
}

fn main() {
    tokio::run(run_server().unit_error().boxed().compat());
}
