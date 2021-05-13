use bitflags::bitflags;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::{collections::VecDeque, task};

use actix::dev::io::WriteHandler;
use actix::{Actor, ActorContext, ActorFuture, AsyncContext, Running, SpawnHandle};
use futures::Sink;

bitflags! {
    struct Flags: u8 {
        const CLOSING = 0b0000_0001;
        const CLOSED = 0b0000_0010;
    }
}

pub trait EmptyBufferHandler
where
    Self: Actor,
    Self::Context: ActorContext,
{
    fn buffer_empty(&mut self, _ctx: &mut Self::Context) {}
}

pub struct SinkWrite<I, S: Sink<I> + Unpin> {
    inner: Rc<RefCell<InnerSinkWrite<I, S>>>,
}

impl<I: 'static, S: Sink<I> + Unpin + 'static> SinkWrite<I, S> {
    pub fn new<A, C>(sink: S, ctxt: &mut C) -> Self
    where
        A: Actor<Context = C> + WriteHandler<S::Error> + EmptyBufferHandler,
        C: AsyncContext<A>,
    {
        let inner = Rc::new(RefCell::new(InnerSinkWrite {
            _i: PhantomData,
            closing_flag: Flags::empty(),
            sink,
            task: None,
            handle: SpawnHandle::default(),
            buffer: VecDeque::new(),
        }));

        let handle = ctxt.spawn(SinkWriteFuture {
            inner: inner.clone(),
            _actor: PhantomData,
        });

        inner.borrow_mut().handle = handle;
        SinkWrite { inner }
    }

    /// Queues an item to be sent to the sink.
    ///
    /// Returns unsent item if sink is closing or closed.
    pub fn write(&mut self, item: I) -> Option<I> {
        if self.inner.borrow().closing_flag.is_empty() {
            self.inner.borrow_mut().buffer.push_back(item);
            self.notify_task();
            None
        } else {
            Some(item)
        }
    }

    pub fn buffer_len(&self) -> usize {
        self.inner.borrow().buffer.len()
    }

    /// Gracefully closes the sink.
    ///
    /// The closing happens asynchronously.
    pub fn close(&mut self) {
        self.inner.borrow_mut().closing_flag.insert(Flags::CLOSING);
        self.notify_task();
    }

    /// Checks if the sink is closed.
    pub fn closed(&self) -> bool {
        self.inner.borrow_mut().closing_flag.contains(Flags::CLOSED)
    }

    fn notify_task(&self) {
        if let Some(task) = &self.inner.borrow().task {
            task.wake_by_ref()
        }
    }

    /// Returns the `SpawnHandle` for this writer.
    pub fn handle(&self) -> SpawnHandle {
        self.inner.borrow().handle
    }
}

struct InnerSinkWrite<I, S: Sink<I>> {
    _i: PhantomData<I>,
    closing_flag: Flags,
    sink: S,
    task: Option<task::Waker>,
    handle: SpawnHandle,

    // buffer of items to be sent so that multiple
    // calls to start_send don't silently skip items
    buffer: VecDeque<I>,
}

struct SinkWriteFuture<I: 'static, S: Sink<I>, A> {
    inner: Rc<RefCell<InnerSinkWrite<I, S>>>,
    _actor: PhantomData<A>,
}

impl<I: 'static, S: Sink<I>, A> ActorFuture for SinkWriteFuture<I, S, A>
where
    S: Sink<I> + Unpin,
    A: Actor + WriteHandler<S::Error> + EmptyBufferHandler,
    A::Context: AsyncContext<A>,
{
    type Output = ();
    type Actor = A;

    fn poll(
        self: Pin<&mut Self>,
        act: &mut A,
        ctxt: &mut A::Context,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let mut trigger_empty = false;

        let this = self.get_mut();
        {
            let inner = &mut this.inner.borrow_mut();

            // ensure sink is ready to receive next item
            match Pin::new(&mut inner.sink).poll_ready(cx) {
                Poll::Ready(Ok(())) => {
                    if let Some(item) = inner.buffer.pop_front() {
                        // send front of buffer to sink
                        let _ = Pin::new(&mut inner.sink).start_send(item);
                        trigger_empty = inner.buffer.is_empty();
                    }
                }
                Poll::Ready(Err(_err)) => {}
                Poll::Pending => {}
            }

            if !inner.closing_flag.contains(Flags::CLOSING) {
                match Pin::new(&mut inner.sink).poll_flush(cx) {
                    Poll::Ready(Err(e)) => {
                        if act.error(e, ctxt) == Running::Stop {
                            act.finished(ctxt);
                            return Poll::Ready(());
                        }
                    }
                    Poll::Ready(Ok(())) => {}
                    Poll::Pending => {}
                }
            } else {
                assert!(!inner.closing_flag.contains(Flags::CLOSED));
                match Pin::new(&mut inner.sink).poll_close(cx) {
                    Poll::Ready(Err(e)) => {
                        if act.error(e, ctxt) == Running::Stop {
                            act.finished(ctxt);
                            return Poll::Ready(());
                        }
                    }
                    Poll::Ready(Ok(())) => {
                        // ensure all items in buffer have been sent before closing
                        if inner.buffer.is_empty() {
                            inner.closing_flag |= Flags::CLOSED;
                            act.finished(ctxt);
                            return Poll::Ready(());
                        }
                    }
                    Poll::Pending => {}
                }
            }

            inner.task.replace(cx.waker().clone());
        }
        if trigger_empty {
            act.buffer_empty(ctxt);
        }

        Poll::Pending
    }
}
