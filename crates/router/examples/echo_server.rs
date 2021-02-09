use futures::prelude::*;

use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::connect;

async fn run_server() {
    let (mut writer, mut reader) = connect(Default::default()).await;

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
        panic!("Unexpected message received")
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
        _ => panic!("Unexpected message received"),
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
}

#[tokio::main]
async fn main() {
    run_server().await;
}
