use futures::prelude::*;
use futures::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use ya_service_bus::{typed as bus, RpcStreamMessage};

#[derive(Serialize, Deserialize)]
struct Ping {}

impl RpcStreamMessage for Ping {
    const ID: &'static str = "ping";
    type Item = String;
    type Error = ();
}
#[actix_rt::test]
async fn test_streaming() {
    fn reference() -> impl Stream<Item = Result<String, ()>> {
        let interval = tokio::time::interval(Duration::from_secs(1));
        tokio_stream::wrappers::IntervalStream::new(interval)
            .flat_map(|_ts| {
                stream::iter(
                    (0..5000)
                        .into_iter()
                        .map(|n| Ok::<_, ()>(format!("hello {n}"))),
                )
            })
            .take(5000 * 3)
    }

    let _ = bus::bind_stream("/local/test", |_p: Ping| reference());

    let x: Vec<_> = bus::service("/local/test")
        .call_streaming(Ping {})
        .map_ok(|v: Result<String, ()>| v.unwrap_or_default())
        .try_collect()
        .await
        .unwrap();
    let y: Vec<_> = reference().try_collect().await.unwrap();

    assert_eq!(x, y)
}
