//! Data cells of the SCQ queue.

use std::mem::MaybeUninit;

use crate::sync::UnsafeCell;

/// One cell of the data array. Unlike the Vyukov queue's slot, it carries no
/// sequence number: ownership comes from holding its index, which is in
/// exactly one place at a time (the free ring, the allocated ring, or a
/// thread that dequeued it).
pub(super) struct DataCell<T> {
    value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> DataCell<T> {
    pub(super) fn new() -> Self {
        Self {
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// # Safety
    /// The caller must hold this cell's index, obtained by dequeuing it from
    /// the free ring, and the cell must be logically empty.
    pub(super) unsafe fn write(&self, value: T) {
        // SAFETY: exclusive access per the caller contract.
        self.value.with_mut(|p| unsafe { (*p).write(value) });
    }

    /// # Safety
    /// The caller must hold this cell's index, obtained by dequeuing it from
    /// the allocated ring (whose `Acquire` synchronises with the producer's
    /// `Release` publish), and the cell must hold an initialised value.
    pub(super) unsafe fn read(&self) -> T {
        // SAFETY: initialised and exclusively owned per the caller contract.
        self.value.with_mut(|p| unsafe { (*p).assume_init_read() })
    }

    /// # Safety
    /// The cell must hold an initialised value and the caller must have
    /// exclusive access to the queue (only called from `Drop`).
    pub(super) unsafe fn drop_in_place(&self) {
        // SAFETY: per the caller contract.
        self.value.with_mut(|p| unsafe { (*p).assume_init_drop() });
    }
}

// SAFETY: a cell owns at most one `T`.
unsafe impl<T: Send> Send for DataCell<T> {}

// SAFETY: access is serialised by index ownership. An index is handed from
// the thread that wrote the cell to the thread that reads it through an
// `AcqRel` publish in the allocated ring and an `Acquire` load by the
// consumer, and back through the free ring the same way. No `&T` is shared.
unsafe impl<T: Send> Sync for DataCell<T> {}
