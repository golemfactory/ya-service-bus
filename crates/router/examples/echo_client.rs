use futures::prelude::*;

use uuid::Uuid;

use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::connect;

async fn run_client() {
    let (mut writer, mut reader) = connect(Default::default()).await;

    let instance_id = Uuid::new_v4().as_bytes().to_vec();
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
