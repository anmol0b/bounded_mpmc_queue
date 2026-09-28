//! One cell of the lock-free ring.

use std::mem::MaybeUninit;

use crate::sync::{AtomicUsize, UnsafeCell};

/// A ring cell: a sequence number plus possibly-uninitialised storage.
///
/// For the position `p` that currently maps to this slot:
/// * `sequence == p`: empty, a producer claiming `p` may write.
/// * `sequence == p + 1`: holds the value pushed at `p`, a consumer may read.
///
/// After the consumer reads, it stores `p + capacity`, which is the "empty"
/// state for the next lap.
pub(crate) struct Slot<T> {
    pub(crate) sequence: AtomicUsize,
    value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> Slot<T> {
    pub(crate) fn new(sequence: usize) -> Self {
        Self {
            sequence: AtomicUsize::new(sequence),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Moves `value` into the slot without dropping the old contents.
    ///
    /// # Safety
    /// The caller must have won the tail CAS for this slot's current position
    /// after observing `sequence == pos`. That makes it the only thread with
    /// access to the storage, and the slot is logically empty.
    #[inline]
    pub(crate) unsafe fn write(&self, value: T) {
        // SAFETY: exclusive access per the caller contract.
        self.value.with_mut(|p| unsafe { (*p).write(value) });
    }

    /// Moves the value out, leaving the slot logically empty.
    ///
    /// # Safety
    /// The caller must have won the head CAS for this slot's current position
    /// after observing `sequence == pos + 1` with `Acquire`. That load
    /// synchronises with the producer's `Release` publish, so the value is
    /// initialised and visible, and no other thread can touch it.
    #[inline]
    pub(crate) unsafe fn read(&self) -> T {
        // SAFETY: initialised and exclusively owned per the caller contract.
        self.value.with_mut(|p| unsafe { (*p).assume_init_read() })
    }

    /// Drops the value in place.
    ///
    /// # Safety
    /// The slot must hold an initialised value and the caller must have
    /// exclusive access to the queue (only called from `Drop`).
    #[inline]
    pub(crate) unsafe fn drop_in_place(&self) {
        // SAFETY: initialised and exclusively owned per the caller contract.
        self.value.with_mut(|p| unsafe { (*p).assume_init_drop() });
    }
}

// SAFETY: a `Slot` owns at most one `T`, so sending it is sending a `T`.
unsafe impl<T: Send> Send for Slot<T> {}

// SAFETY: shared access to the storage is serialised by the sequence
// protocol. Only the thread that wins the head or tail CAS for a position
// touches the value, and the Release store / Acquire load on `sequence` orders
// the producer's write before the consumer's read and the consumer's read
// before the next lap's write. Values move between threads, so `T: Send` is
// required; `T: Sync` is not, because no `&T` is ever shared.
unsafe impl<T: Send> Sync for Slot<T> {}
