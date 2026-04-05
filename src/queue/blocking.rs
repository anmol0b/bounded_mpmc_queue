use crate::queue::ring_buffer::RingBuffer;
use std::sync::{Condvar, Mutex};

pub struct BlockingQueue<T> {
    buffer: Mutex<RingBuffer<T>>,
    not_full: Condvar,
    not_empty: Condvar,
}

impl<T> BlockingQueue<T> {
    pub fn new(capacity: usize) -> BlockingQueue<T> {
        BlockingQueue {
            buffer: Mutex::new(RingBuffer::new(capacity)),
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
        }
    }
    pub fn push(&self, item: T) {
        let mut guard = self.buffer.lock().unwrap();
        while guard.is_full() {
            guard = self.not_full.wait(guard).unwrap();
        }
        guard.push(item);
        self.not_empty.notify_one();
    }
    pub fn pop(&self) -> T {
        let mut guard = self.buffer.lock().unwrap();
        while guard.is_empty() {
            guard = self.not_empty.wait(guard).unwrap();
        }
        let item = guard.pop().unwrap();
        self.not_full.notify_one();
        item
    }
}
