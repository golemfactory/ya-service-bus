use futures::StreamExt;
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::env;
use std::ops::Rem;
use std::time::Duration;
use ya_service_bus::sequence::{Sequence, Sequenced, SequencedStream};

struct Chunk<N: Sequence>(N);

impl<N: Sequence> Sequenced<N> for Chunk<N> {
    fn seq(&self) -> N {
        self.0
    }
}

async fn test<N, I>(
    name: &'static str,
    src: I,
    total: usize,
    chunk_size: usize,
    capacity: usize,
    step: N,
) where
    N: Sequence + Rem<Output = N> + 'static,
    I: Iterator<Item = N> + Clone + 'static,
{
    log::warn!("Starting test ({:?}) in 2s", name);
    log::info!("\ttotal len: {}", total);
    log::info!("\tchunk_size: {}", chunk_size);
    log::info!("\tbuffer len: {}", capacity);

    tokio::time::delay_for(Duration::from_secs(2)).await;

    let stream = futures::stream::iter(src.cycle().map(|s| Ok::<_, ()>(Chunk(s))).take(total))
        .chunks(chunk_size)
        .map(|mut v| {
            v.shuffle(&mut thread_rng());
            futures::stream::iter(v.into_iter())
        })
        .flatten();

    SequencedStream::<_, _, _, N>::new(stream, capacity)
        .fold(N::default(), |expected, r| async move {
            let item = r.unwrap();
            if item.seq().rem(step).eq(&N::default()) {
                log::info!("step: {:?}", item.seq());
            }
            assert_eq!(item.seq(), expected);
            expected.next()
        })
        .await;
}

#[actix_rt::main]
async fn main() {
    env::set_var("RUST_LOG", env::var("RUST_LOG").unwrap_or("trace".into()));
    env_logger::init();

    test::<u8, _>(
        "u8",
        0..=u8::max_value(),
        u8::max_value() as usize * 353,
        11,
        32,
        100,
    )
    .await;

    test::<u16, _>(
        "u16",
        0..=u16::max_value(),
        u16::max_value() as usize * 7,
        128,
        384,
        10000,
    )
    .await;

    test::<u32, _>(
        "u32",
        0..=u32::max_value(),
        u32::max_value() as usize * 3,
        128,
        384,
        100000,
    )
    .await;
}
