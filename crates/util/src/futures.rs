#![allow(unused)]

use futures::prelude::*;
use futures::task::{Context, Poll};
use std::marker::PhantomData;
use std::pin::Pin;

#[pin_project::pin_project]
pub struct Flatten<F, E> {
    #[pin]
    inner: F,
    _marker: PhantomData<E>,
}

pub trait IntoFlatten<E> {
    fn flatten_fut(self) -> Flatten<Self, E>
    where
        Self: Sized,
    {
        Flatten {
            inner: self,
            _marker: PhantomData,
        }
    }
}

impl<Item, Error, F: TryFuture<Ok = Result<Item, Error>>> IntoFlatten<Error> for F {}

impl<Item, Error, F: TryFuture<Ok = Result<Item, Error>>> Future for Flatten<F, Error>
where
    Error: From<F::Error>,
{
    type Output = Result<Item, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.inner.try_poll(cx) {
            Poll::Ready(Ok(v)) => Poll::Ready(v),
            Poll::Ready(Err(e)) => Poll::Ready(Err(Error::from(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}
