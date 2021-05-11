use actix::prelude::*;
use futures::Stream;
use pin_project::*;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

pub trait InputHandler<I>
where
    Self: Actor,
    Self::Context: AsyncContext<Self>,
{
    fn handle(
        &mut self,
        item: I,
        ctx: &mut Self::Context,
    ) -> Pin<Box<dyn ActorFuture<Output = (), Actor = Self>>>;

    fn finished(&mut self, ctx: &mut Self::Context) {
        ctx.stop()
    }

    fn started(&mut self, _ctx: &mut Self::Context) {}

    fn add_stream<S>(fut: S, ctx: &mut Self::Context) -> SpawnHandle
    where
        Self::Context: AsyncContext<Self>,
        S: Stream<Item = I> + 'static,
        I: 'static,
    {
        if ctx.state() == ActorState::Stopped {
            log::error!("Context::add_stream called for stopped actor.");
            SpawnHandle::default()
        } else {
            ctx.spawn(ActorStream::new(fut))
        }
    }
}

#[pin_project]
struct ActorStream<A: InputHandler<M>, M, S>
where
    A: Actor + InputHandler<M>,
    A::Context: AsyncContext<A>,
{
    #[pin]
    stream: S,
    processing: Option<Pin<Box<dyn ActorFuture<Output = (), Actor = A>>>>,
    started: bool,
    act: PhantomData<A>,
    msg: PhantomData<M>,
}

impl<A: InputHandler<M>, M, S> ActorStream<A, M, S>
where
    A: Actor + InputHandler<M>,
    A::Context: AsyncContext<A>,
{
    fn new(stream: S) -> Self {
        ActorStream {
            stream,
            processing: None,
            started: false,
            act: PhantomData::default(),
            msg: PhantomData::default(),
        }
    }
}

impl<A, M, S> ActorFuture for ActorStream<A, M, S>
where
    S: Stream<Item = M>,
    A: Actor + InputHandler<M>,
    A::Context: AsyncContext<A>,
{
    type Output = ();
    type Actor = A;

    fn poll(
        self: Pin<&mut Self>,
        act: &mut A,
        ctx: &mut A::Context,
        task: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let mut this = self.project();

        if !*this.started {
            <A as InputHandler<M>>::started(act, ctx);
            *this.started = true;
        }

        if let Some(fut) = this.processing.as_mut() {
            match fut.as_mut().poll(act, ctx, task) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(()) => (),
            }
            *this.processing = None
        }

        match this.stream.as_mut().poll_next(task) {
            Poll::Ready(Some(msg)) => {
                *this.processing = Some(A::handle(act, msg, ctx));
                if !ctx.waiting() {
                    // Let the future's context know that this future might be polled right the way
                    task.waker().wake_by_ref();
                }
                Poll::Pending
            }
            Poll::Ready(None) => {
                A::finished(act, ctx);
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
