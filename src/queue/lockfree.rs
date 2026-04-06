use crate::sync::backoff::Backoff;
use crate::utils::cache_pad::CachePadded;
use crate::{queue::slot::Slot, traits::bounded_queue::BoundedQueue};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct LockFreeQueue<T> {
    slots: Box<[Slot<T>]>,
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    capacity: usize,
}

impl<T> LockFreeQueue<T> {
    pub fn new(capacity: usize) -> Self {
        let slots = (0..capacity)
            .map(|i| Slot::new(i))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        LockFreeQueue {
            slots,
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
            capacity,
        }
    }
    pub fn try_push(&self, item: T) -> Result<(), T> {
        let pos = self.tail.load(Ordering::Relaxed);
        let slot = &self.slots[pos & (self.capacity - 1)];
        let seq = slot.sequence.load(Ordering::Acquire);
        if seq == pos {
            match self
                .tail
                .compare_exchange(pos, pos + 1, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => {
                    unsafe {
                        *slot.data.get() = Some(item);
                    }
                    slot.sequence.store(pos + 1, Ordering::Release);
                    return Ok(());
                }
                Err(_) => {
                    return Err(item);
                }
            }
        } else {
            return Err(item);
        }
    }
    pub fn try_pop(&self) -> Option<T> {
        let pos = self.head.load(Ordering::Relaxed);
        let slot = &self.slots[pos & (self.capacity - 1)];
        let seq = slot.sequence.load(Ordering::Acquire);
        if seq == pos + 1 {
            match self
                .head
                .compare_exchange(pos, pos + 1, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => {
                    let item = unsafe { (*slot.data.get()).take() };
                    slot.sequence
                        .store(pos.wrapping_add(self.capacity), Ordering::Release);
                    return item;
                }
                Err(_) => return None,
            }
        } else {
            return None;
        }
    }
    pub fn push(&self, mut item: T) {
        let mut backoff = Backoff::new();
        loop {
            match self.try_push(item) {
                Ok(()) => return,
                Err(returned) => {
                    item = returned;
                    backoff.spin();
                }
            }
        }
    }
    pub fn pop(&self) -> T {
        let mut backoff = Backoff::new();
        loop {
            match self.try_pop() {
                Some(item) => return item,
                None => backoff.spin(),
            }
        }
    }
}
impl<T: Send> BoundedQueue<T> for LockFreeQueue<T> {
    fn new(capacity: usize) -> Self {
        LockFreeQueue::new(capacity)
    }
    fn push(&self, item: T) {
        self.push(item)
    }
    fn pop(&self) -> T {
        self.pop()
    }
    fn try_push(&self, item: T) -> Result<(), T> {
        self.try_push(item)
    }
    fn try_pop(&self) -> Option<T> {
        self.try_pop()
    }
}
