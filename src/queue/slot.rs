use std::{cell::UnsafeCell, sync::atomic::AtomicUsize};

pub struct Slot<T> {
    pub sequence: AtomicUsize,
    pub data: UnsafeCell<Option<T>>,
}
impl<T> Slot<T> {
    pub fn new(sequence: usize) -> Self {
        Slot {
            sequence: AtomicUsize::new(sequence),
            data: UnsafeCell::new(None),
        }
    }
}
// SAFETY: access to `data` is gated by the sequence protocol; see Sync below.
unsafe impl<T: Send> Send for Slot<T> {}
// SAFETY: only the thread that wins the head/tail CAS for a position touches
// `data`, and the Release/Acquire pair on `sequence` orders those accesses.
unsafe impl<T: Send> Sync for Slot<T> {}
