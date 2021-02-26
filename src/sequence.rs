use futures::stream::Peekable;
use futures::task::Poll;
use futures::{Stream, StreamExt};
use std::fmt::Debug;
use std::pin::Pin;

pub const MAX_SEQUENCE_BUFFER_SIZE: usize = 256;

pub trait Sequence: Eq + Ord + Sized + Copy + Debug + Default + Unpin {
    fn next(&self) -> Self;
}
pub trait Sequenced<N: Sequence>: Unpin {
    fn seq(&self) -> N;
}

macro_rules! impl_sequence {
    ($ty:ty) => {
        impl Sequence for $ty {
            fn next(&self) -> Self {
                self.overflowing_add(1).0
            }
        }
    };
}

impl_sequence!(u8);
impl_sequence!(u16);
impl_sequence!(u32);
impl_sequence!(u64);
impl_sequence!(u128);

#[derive(Copy, Clone, Debug)]
enum BufferState {
    /// Normal operation
    Normal,
    /// Split buffer into 2 parts:
    /// current | overflow sequence numbers
    Split(usize),
    /// Drain contents and skip buffering
    Drain,
}

pub struct SequencedStream<S, E, Q, N>
where
    S: Stream<Item = Result<Q, E>> + 'static,
    E: 'static,
    Q: Sequenced<N> + 'static,
    N: Sequence + 'static,
{
    stream: Peekable<S>,
    buffer: Vec<Q>,
    state: BufferState,
    next_seq: N,
}

impl<S, E, Q, N> SequencedStream<S, E, Q, N>
where
    S: Stream<Item = Result<Q, E>> + 'static,
    E: 'static,
    Q: Sequenced<N> + 'static,
    N: Sequence + 'static,
{
    pub fn new(stream: S, capacity: usize) -> Self {
        Self {
            stream: stream.peekable(),
            buffer: Vec::with_capacity(capacity),
            state: BufferState::Normal,
            next_seq: Default::default(),
        }
    }

    fn enqueue(&mut self, item: Q) -> Option<Q> {
        if self.buffer.len() == self.buffer.capacity() {
            log::warn!("Stream sequence buffer is full, draining and entering pass-through mode");
            self.state = BufferState::Drain;
            return Some(item);
        }

        let is_overflow = item.seq() < self.next_seq;
        let (start, end) = match self.state {
            BufferState::Split(idx) => {
                if is_overflow {
                    (idx, self.buffer.len())
                } else {
                    self.state = BufferState::Split(idx + 1);
                    (0, idx)
                }
            }
            _ => {
                if is_overflow {
                    self.state = BufferState::Split(self.buffer.len());
                    return self.enqueue(item);
                }
                (0, self.buffer.len())
            }
        };

        let idx = (&mut self.buffer[start..end])
            .binary_search_by_key(&item.seq(), |s| s.seq())
            .unwrap_or_else(|i| i);

        self.buffer.insert(start + idx, item);
        None
    }

    #[inline(always)]
    fn has_next(&self) -> bool {
        self.buffer
            .get(0)
            .map(|c| c.seq() == self.next_seq)
            .unwrap_or(false)
    }

    #[inline(always)]
    fn get_next(&mut self) -> Q {
        let chunk = self.buffer.remove(0);
        if let BufferState::Split(n) = &self.state {
            if *n >= 1 {
                self.state = BufferState::Split(n - 1);
            }
        };
        chunk
    }

    #[inline(always)]
    fn is_overflow(&mut self, item: &Q) {
        if item.seq().next() == N::default() {
            if let BufferState::Split(_) = self.state {
                self.state = BufferState::Normal;
            }
        }
    }

    #[inline(always)]
    fn advance(&mut self, cx: &mut std::task::Context<'_>) {
        self.next_seq = self.next_seq.next();
        if self.has_next() {
            cx.waker().wake_by_ref();
        }
    }
}

impl<S, E, Q, N> Stream for SequencedStream<S, E, Q, N>
where
    S: Stream<Item = Result<Q, E>> + Unpin + 'static,
    E: 'static,
    Q: Sequenced<N> + 'static,
    N: Sequence + 'static,
{
    type Item = Result<Q, E>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let &BufferState::Drain = &self.state {
            let chunk = match self.buffer.len() {
                0 => return Pin::new(&mut self.stream).poll_next(cx),
                _ => self.get_next(),
            };
            cx.waker().wake_by_ref();
            return Poll::Ready(Some(Ok(chunk)));
        } else if self.has_next() {
            let item = self.get_next();
            self.is_overflow(&item);
            self.advance(cx);
            return Poll::Ready(Some(Ok(item)));
        }

        if let Poll::Ready(result) = Pin::new(&mut self.stream).poll_next(cx) {
            match result {
                Some(Ok(item)) => {
                    if item.seq() == self.next_seq {
                        self.is_overflow(&item);
                        self.advance(cx);
                        return Poll::Ready(Some(Ok(item)));
                    } else if let Some(i) = self.as_mut().enqueue(item) {
                        cx.waker().wake_by_ref();
                        return Poll::Ready(Some(Ok(i)));
                    }
                }
                res => return Poll::Ready(res),
            }
        }

        if self.has_next() {
            cx.waker().wake_by_ref();
        } else if let Poll::Ready(Some(_)) = Pin::new(&mut self.stream).poll_peek(cx) {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}
