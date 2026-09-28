//! A plain single-threaded ring buffer, used under the lock of `BlockingQueue`.

#[derive(Debug)]
pub(crate) struct RingBuffer<T> {
    buffer: Box<[Option<T>]>,
    head: usize,
    tail: usize,
    len: usize,
}

impl<T> RingBuffer<T> {
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be non-zero");
        Self {
            buffer: (0..capacity).map(|_| None).collect(),
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.buffer.len()
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn is_full(&self) -> bool {
        self.len == self.capacity()
    }

    /// Caller must check `!is_full()` first.
    pub(crate) fn push(&mut self, item: T) {
        debug_assert!(!self.is_full());
        debug_assert!(self.buffer[self.tail].is_none());
        self.buffer[self.tail] = Some(item);
        self.tail = (self.tail + 1) % self.capacity();
        self.len += 1;
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        let item = self.buffer[self.head].take()?;
        self.head = (self.head + 1) % self.capacity();
        self.len -= 1;
        Some(item)
    }
}
