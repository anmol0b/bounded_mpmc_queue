//! One-shot signals that a job has finished.

use std::sync::PoisonError;

use crate::sync::{Arc, AtomicUsize, Condvar, Mutex, Ordering};

/// Signals completion of a job.
pub(super) trait Latch {
    /// Marks the latch as set. After this the setter must not touch the
    /// latch (or the job containing it) again: the waiter may free it.
    fn set(this: &Self);
}

/// A flag the waiting worker polls while it helps with other work. Used by
/// `join`, where the waiter is a worker thread that keeps executing jobs.
pub(super) struct SpinLatch {
    state: AtomicUsize,
}

impl SpinLatch {
    pub(super) fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
        }
    }

    /// Acquire pairs with `set`'s Release: the job's result is visible.
    pub(super) fn probe(&self) -> bool {
        self.state.load(Ordering::Acquire) == 1
    }
}

impl Latch for SpinLatch {
    fn set(this: &Self) {
        this.state.store(1, Ordering::Release);
    }
}

/// A latch an outside thread can block on. Used by `install`.
///
/// The flag and condvar are behind an `Arc` the setter clones *before*
/// setting: the waiter lives in another thread's frame and may return (and
/// free the latch) the moment it sees the flag, so the setter must not unlock
/// or notify through the latch itself.
pub(super) struct LockLatch {
    shared: Arc<(Mutex<bool>, Condvar)>,
}

impl LockLatch {
    pub(super) fn new() -> Self {
        Self {
            shared: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    pub(super) fn wait(&self) {
        let (lock, cv) = &*self.shared;
        let mut set = lock.lock().unwrap_or_else(PoisonError::into_inner);
        while !*set {
            set = cv.wait(set).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Latch for LockLatch {
    fn set(this: &Self) {
        let shared = Arc::clone(&this.shared);
        // From here on only `shared` is used, never `this`.
        let (lock, cv) = &*shared;
        *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
        cv.notify_all();
    }
}
