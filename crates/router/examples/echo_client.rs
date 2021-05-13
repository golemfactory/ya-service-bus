use futures::prelude::*;

use uuid::Uuid;

use structopt::StructOpt;
use ya_sb_proto::codec::GsbMessage;
use ya_sb_proto::*;
use ya_sb_router::connect;

async fn run_client(args: Args) {
    let (mut writer, mut reader) = connect(Default::default()).await;

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

#[derive(StructOpt)]
struct Args {
    #[structopt(long, short)]
    delay: Option<u64>,
    #[structopt(long, short)]
    count: Option<u64>,
}

#[tokio::main]
async fn main() {
    run_client(Args::from_args()).await;
}
