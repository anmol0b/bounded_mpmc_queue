use std::{cell::UnsafeCell, sync::atomic::AtomicUsize, usize};

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
unsafe impl<T: Send> Send for Slot<T> {}
unsafe impl<T: Send> Sync for Slot<T> {}
