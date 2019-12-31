use failure::Error;
use futures::{FutureExt, TryFutureExt};
use serde::{Deserialize, Serialize};
use ya_service_bus::{typed as bus, RpcMessage, RpcStreamMessage};

#[derive(Serialize, Deserialize)]
struct Ping(String);

impl RpcMessage for Ping {
    const ID: &'static str = "ping";
    type Item = String;
    type Error = ();
}

impl RpcStreamMessage for Ping {
    const ID: &'static str = "ping";
    type Item = String;
    type Error = ();
}

async fn server() -> Result<(), Error> {
    log::info!("starting");
    let (tx, rx) = futures::channel::oneshot::channel::<()>();

    let mut txh = Some(tx);
    let quit = move |p: Ping| {
        let mut tx = txh.take().unwrap();
        async move {
            eprintln!("quit!!");
            let _ = tx.send(());
            {
                Ok("quit".to_string())
            }
        }
    };

    let _ = bus::bind("/local/test", |p: Ping| {
        async move {
            eprintln!("got ping: {:?}", p.0);
            Ok(format!("pong {}", p.0))
        }
    });
    let _ = bus::bind("/local/quit", quit);
    let _ = bus::bind_streaming("/local/stream", |p: Ping| {
        let s = async_stream::stream! {
            for i in 0..3 {
                yield Ok(format!("ack {}", i))
            }
        };
        Box::pin(s)
    });

    let _ = rx.await;
    Ok(())
}

fn main() -> Result<(), Error> {
    env_logger::init();
    actix::System::new("w").block_on(server().boxed_local().compat())
}
