//! Parking for threads that ran out of spin budget.

use std::sync::PoisonError;
use std::time::Instant;

use crate::sync::{AtomicUsize, Condvar, Mutex, MutexGuard, Ordering::SeqCst};
use crate::utils::CachePadded;

/// A set of parked threads waiting for a queue condition to become true.
///
/// # Protocol
///
/// The condition itself lives in the queue's atomics, not behind `lock`. The
/// mutex protects no data; it exists only so that "recheck the condition,
/// then block" is atomic with respect to [`notify_one`](Self::notify_one).
///
/// A waiter registers (`waiters += 1`, `SeqCst`), then re-evaluates `ready`,
/// whose loads are `SeqCst`. A notifier performs its state change with a
/// `SeqCst` RMW (the head/tail CAS), then loads `waiters` with `SeqCst`.
/// All four operations sit in the single `SeqCst` total order, so it is
/// impossible for the notifier to see zero waiters *and* the waiter to miss
/// the state change: the store-buffering outcome is forbidden. Whichever
/// happens first, either the waiter sees the new state and never blocks, or
/// the notifier sees the waiter and takes the lock to wake it.
///
/// The remaining window, between the waiter's recheck and its block, is
/// closed by the mutex: the waiter holds it from registration until
/// `Condvar::wait` atomically releases it, and the notifier must acquire it
/// before calling `notify_one`.
///
/// Downgrading any of the four `SeqCst` operations to `Acquire`/`Release`
/// reintroduces the lost wakeup; the loom suite catches it.
#[derive(Debug)]
pub(crate) struct WaitQueue {
    /// Threads currently inside [`wait_until`](Self::wait_until). Read on
    /// every successful queue operation, written only on the slow path, so it
    /// gets its own cache line.
    waiters: CachePadded<AtomicUsize>,
    lock: Mutex<()>,
    cv: Condvar,
}

impl WaitQueue {
    pub(crate) fn new() -> Self {
        Self {
            waiters: CachePadded::new(AtomicUsize::new(0)),
            lock: Mutex::new(()),
            cv: Condvar::new(),
        }
    }

    fn guard(&self) -> MutexGuard<'_, ()> {
        // The mutex guards `()`, so a poisoned lock carries no broken invariant.
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes one parked thread, if any. Costs a single `SeqCst` load when
    /// nobody is parked, which is the common case on the hot path.
    #[inline]
    pub(crate) fn notify_one(&self) {
        if self.waiters.load(SeqCst) == 0 {
            return;
        }
        let _guard = self.guard();
        self.cv.notify_one();
    }

    /// Wakes every parked thread. Used by `close`, so it never consults the
    /// waiter count.
    pub(crate) fn notify_all(&self) {
        let _guard = self.guard();
        self.cv.notify_all();
    }

    /// Blocks until `ready()` returns `true` or `deadline` passes.
    ///
    /// Returns `true` if `ready()` was observed to be `true`. `ready` is always
    /// evaluated before the deadline check, so a wakeup that races with the
    /// timeout resolves in favour of the caller making progress.
    pub(crate) fn wait_until(
        &self,
        mut ready: impl FnMut() -> bool,
        deadline: Option<Instant>,
    ) -> bool {
        let mut guard = self.guard();
        self.waiters.fetch_add(1, SeqCst);
        let observed = loop {
            if ready() {
                break true;
            }
            match deadline {
                None => {
                    guard = self.cv.wait(guard).unwrap_or_else(PoisonError::into_inner);
                }
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        break false;
                    }
                    guard = self
                        .cv
                        .wait_timeout(guard, deadline - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
            }
        };
        self.waiters.fetch_sub(1, SeqCst);
        drop(guard);
        observed
    }
}
