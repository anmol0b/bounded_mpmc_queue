//! Futex-based parking.

use std::time::Instant;

use crate::sync::futex::Futex;
use crate::sync::{AtomicUsize, Ordering::Acquire, Ordering::Relaxed, Ordering::SeqCst, fence};
use crate::utils::CachePadded;

/// A set of parked threads waiting for a queue condition to become true.
///
/// Waiters sleep on a 32-bit `epoch` word. Notifiers bump the word, then wake.
/// The kernel's compare-and-sleep takes over the role the mutex plays in the
/// fallback implementation: a notification that lands between a waiter's
/// re-check and its sleep has already changed the word, so the sleep does not
/// happen.
///
/// # Protocol
///
/// Waiter (`wait_until`):
///
/// 1. register: `waiters += 1`;
/// 2. `seen = epoch.load(Acquire)` — **before** the re-check;
/// 3. re-check the queue with an `AcqRel` RMW on the notifier's word;
/// 4. if not ready: `sleepers += 1`, `fence(SeqCst)`, `futex_wait(epoch,
///    seen)`, `sleepers -= 1`; then go to 2.
///
/// Notifier (`notify_one`), after its `AcqRel` CAS on that word:
///
/// 1. if `waiters == 0` (`Relaxed`), stop: this is the hot path;
/// 2. `epoch.fetch_add(1, Release)`;
/// 3. `fence(SeqCst)`, then if `sleepers != 0`, `futex_wake(epoch, 1)`.
///
/// # Why no wakeup is lost
///
/// The waiter's re-check (W3) and the notifier's CAS (N1) are RMWs on the same
/// word, so one precedes the other in its modification order.
///
/// * N1 first: W3 reads it (an RMW reads the latest value); the waiter sees
///   the new state and does not sleep.
/// * W3 first: N1 reads from W3's release sequence and synchronises with it.
///   So registration happens-before the notifier's `waiters` load, which sees
///   it, and the notifier bumps. Also, `seen` (W2) happens-before the bump, so
///   by read-write coherence `seen` is an older epoch than the bump's. When
///   the waiter's `futex_wait` compares, either the bump is visible and it
///   returns at once, or the waiter is already asleep and the wake reaches it.
///
/// # Skipping the system call
///
/// A registered waiter is often still awake: spinning, re-checking, or just
/// woken by an earlier notification. Waking the futex anyway costs a system
/// call per queue operation, which measurably cut throughput under churn.
/// So the notifier wakes only if `sleepers != 0`.
///
/// That creates a second store-buffering race: the notifier writes `epoch`
/// then reads `sleepers`; the waiter writes `sleepers` then (in the kernel)
/// reads `epoch`. The two `SeqCst` fences sit in the single total order of
/// fences. If the waiter's fence is first, the notifier's `sleepers` load
/// sees the increment and wakes. If the notifier's fence is first, the
/// kernel's compare sees the bump and does not sleep. Both fences are on the
/// slow path; a notifier only reaches its fence when a thread is registered.
///
/// `close` bumps unconditionally and wakes everyone. Its `Release` bump pairs
/// with the waiter's `Acquire` read of `seen`, so after the waiter's next
/// iteration it sees the closed flag even through `Relaxed` loads.
///
/// The epoch is 32 bits. A waiter could sleep wrongly only if exactly a
/// multiple of 2^32 bumps landed between its read of `seen` and the kernel's
/// compare, a window of a few instructions; even then the next notification
/// wakes it. Rust's own futex `Condvar` accepts the same window.
///
/// Every other return from `futex_wait`, whether a timeout, a signal, a wake
/// meant for another waiter or a spurious one, is harmless: the loop re-reads
/// the epoch and re-checks.
#[derive(Debug)]
pub(crate) struct WaitQueue {
    inner: CachePadded<Inner>,
}

/// Both fields are cold while nobody is parked, so they share a line.
#[derive(Debug)]
struct Inner {
    /// Threads inside `wait_until`, asleep or not.
    waiters: AtomicUsize,
    /// Threads about to sleep or asleep in the kernel. Lets a notifier skip
    /// the wake system call when every registered waiter is still awake.
    sleepers: AtomicUsize,
    epoch: Futex,
}

impl WaitQueue {
    pub(crate) fn new() -> Self {
        Self {
            inner: CachePadded::new(Inner {
                waiters: AtomicUsize::new(0),
                sleepers: AtomicUsize::new(0),
                epoch: Futex::new(),
            }),
        }
    }

    /// Wakes one parked thread, if any. Costs a single `Relaxed` load when
    /// nobody is parked.
    ///
    /// The caller must have published its state change with an `AcqRel` RMW
    /// on the word the waiters' `ready` check reads with an RMW.
    #[inline]
    pub(crate) fn notify_one(&self) {
        if self.inner.waiters.load(Relaxed) == 0 {
            return;
        }
        self.inner.epoch.bump();
        // Pairs with the fence in `wait_until`: see "Skipping the system call".
        fence(SeqCst);
        if self.inner.sleepers.load(Relaxed) != 0 {
            self.inner.epoch.wake_one();
        }
    }

    /// `true` if a thread may be parked or about to park. A `Relaxed`
    /// snapshot, for callers whose wakeups are best-effort (see the pool).
    #[inline]
    pub(crate) fn has_waiters(&self) -> bool {
        self.inner.waiters.load(Relaxed) != 0
    }

    /// Wakes every parked thread. Used by `close`, so it never consults the
    /// waiter count.
    pub(crate) fn notify_all(&self) {
        self.inner.epoch.bump();
        self.inner.epoch.wake_all();
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
        self.inner.waiters.fetch_add(1, Relaxed);
        let observed = loop {
            // Must precede the re-check: see the type-level docs.
            let seen = self.inner.epoch.load(Acquire);
            if ready() {
                break true;
            }
            let timeout = match deadline {
                None => None,
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        break false;
                    }
                    Some(deadline - now)
                }
            };
            self.inner.sleepers.fetch_add(1, Relaxed);
            // Pairs with the fence in `notify_one`.
            fence(SeqCst);
            self.inner.epoch.wait(seen, timeout);
            self.inner.sleepers.fetch_sub(1, Relaxed);
        };
        self.inner.waiters.fetch_sub(1, Relaxed);
        observed
    }
}
