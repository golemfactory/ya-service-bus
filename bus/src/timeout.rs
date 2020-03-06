use futures::future::{Either, Future, Map};
use futures::FutureExt;
use std::time::Duration;
use tokio::time::{timeout, Elapsed, Timeout};

pub trait IntoDuration {
    fn into_duration(self) -> Duration;
}

impl IntoDuration for f32 {
    #[inline(always)]
    fn into_duration(self) -> Duration {
        Duration::from_secs_f32(self)
    }
}

// FIXME: remove it
impl IntoDuration for u32 {
    #[inline(always)]
    fn into_duration(self) -> Duration {
        Duration::from_millis(self as u64)
    }
}

impl IntoDuration for Duration {
    #[inline(always)]
    fn into_duration(self) -> Duration {
        self
    }
}

type MapType<F> = Map<F, fn(<F as Future>::Output) -> Result<<F as Future>::Output, Elapsed>>;

pub trait IntoTimeoutFuture<D>: Future
where
    Self: Future + Sized,
    D: IntoDuration,
{
    fn timeout(self, duration: Option<D>) -> Either<Timeout<Self>, MapType<Self>>;
}

impl<F, D> IntoTimeoutFuture<D> for F
where
    Self: Future + Sized,
    D: IntoDuration,
{
    fn timeout(self, duration: Option<D>) -> Either<Timeout<Self>, MapType<Self>> {
        match duration {
            Some(d) => Either::Left(timeout(d.into_duration(), self)),
            None => Either::Right(self.map(|v| Result::Ok(v))),
        }
    }
}
