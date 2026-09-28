//! `Mutex` + `Condvar` parking: the portable fallback.

use std::sync::PoisonError;
use std::time::Instant;

use crate::sync::{AtomicUsize, Condvar, Mutex, MutexGuard, Ordering::Relaxed};
use crate::utils::CachePadded;

/// A set of parked threads waiting for a queue condition to become true.
///
/// # Protocol
///
/// The condition itself lives in the queue's atomics, not behind `lock`. The
/// mutex protects no data; it exists only so that "recheck the condition,
/// then block" is atomic with respect to [`notify_one`](Self::notify_one).
///
/// The hazard is a lost wakeup: the waiter checks the queue (empty), the
/// notifier makes it non-empty and checks `waiters` (zero), and the waiter
/// then blocks forever. It is ruled out as follows, taking a parked consumer
/// and a producer as the example (the producer side mirrors it on `head`):
///
/// 1. The waiter registers (`waiters += 1`), then re-checks the queue with an
///    `AcqRel` read-modify-write on `tail` (`fetch_add(0)`), not a load.
/// 2. The notifier advances `tail` with an `AcqRel` CAS, then loads
///    `waiters` (`Relaxed`).
///
/// Both are RMWs on `tail`, so one precedes the other in its modification
/// order. If the notifier's CAS is first, the waiter's RMW reads its value
/// (an RMW always reads the latest value) and the waiter does not block. If
/// the waiter's RMW is first, the notifier's CAS reads from that RMW's
/// release sequence and synchronises with it, so the registration happens
/// before the notifier's load of `waiters`, which must see it.
///
/// The remaining window, between the waiter's recheck and its block, is
/// closed by the mutex: the waiter holds it from registration until
/// `Condvar::wait` atomically releases it, and the notifier must acquire it
/// before calling `notify_one`.
///
/// An earlier version paired `SeqCst` loads with `SeqCst` RMWs instead. That
/// is also correct in C11, but loom treats `SeqCst` accesses as `AcqRel` and
/// could not verify it. This formulation is checked by loom, and it keeps
/// `SeqCst` off the hot path: a successful push or pop pays one `Relaxed`
/// load when nobody is parked.
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

    /// Wakes one parked thread, if any. Costs a single `Relaxed` load when
    /// nobody is parked, which is the common case on the hot path.
    ///
    /// The caller must have published its state change with an `AcqRel` RMW
    /// on the word the waiters' `ready` check reads with an RMW.
    #[inline]
    pub(crate) fn notify_one(&self) {
        if self.waiters.load(Relaxed) == 0 {
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
    /// `ready` must observe the queue state through an `AcqRel` RMW (see the
    /// type-level docs). Registration is ordered before it by program order.
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
        self.waiters.fetch_add(1, Relaxed);
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
        self.waiters.fetch_sub(1, Relaxed);
        drop(guard);
        observed
    }
}
