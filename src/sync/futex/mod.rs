//! A 32-bit word threads can block on until it changes: a futex.
//!
//! # Contract
//!
//! `wait(expected, timeout)` atomically, with respect to `wake_*` on the same
//! word, checks that the word still equals `expected` and, if so, sleeps
//! until woken, until the timeout, or spuriously. A change to the word that is
//! sequenced before a `wake_*` call is therefore either seen by the compare,
//! or the waiter is already asleep and the wake reaches it. The compare gives
//! the caller no memory ordering; callers must re-check their condition after
//! every return, for any reason.
//!
//! # Backends
//!
//! * Linux and Android: the `futex(2)` system call.
//! * macOS: `__ulock_wait` / `__ulock_wake`. They are private but have had a
//!   stable ABI since macOS 10.12; libc++ implements `std::atomic::wait` on
//!   them. Rust's standard library avoids them only because App Store review
//!   rejects private symbols.
//! * loom: a model built from loom's `Mutex` and `Condvar` (see [`Futex`]).
//!
//! Everything else uses the `Mutex` + `Condvar` wait queue instead of this
//! module (see `sync::wait_queue`).

use std::time::Duration;

use crate::sync::{AtomicU32, Ordering};

#[cfg(all(not(loom), any(target_os = "linux", target_os = "android")))]
#[path = "linux.rs"]
mod imp;

#[cfg(all(not(loom), target_os = "macos"))]
#[path = "macos.rs"]
mod imp;

/// A futex word. See the module docs for the contract.
///
/// Under loom the kernel is modelled with a mutex and condition variable:
/// `wait` locks, compares, and waits on the condvar; `wake_*` locks and
/// notifies. That is a faithful model of compare-and-enqueue atomicity, but it
/// also adds a happens-before edge from waker to woken thread that a real
/// futex does not provide. The parking proof therefore never relies on one.
#[derive(Debug)]
pub(crate) struct Futex {
    word: AtomicU32,
    #[cfg(loom)]
    lock: crate::sync::Mutex<()>,
    #[cfg(loom)]
    cv: crate::sync::Condvar,
}

impl Futex {
    pub(crate) fn new() -> Self {
        Self {
            word: AtomicU32::new(0),
            #[cfg(loom)]
            lock: crate::sync::Mutex::new(()),
            #[cfg(loom)]
            cv: crate::sync::Condvar::new(),
        }
    }

    #[inline]
    pub(crate) fn load(&self, order: Ordering) -> u32 {
        self.word.load(order)
    }

    /// Changes the word (wrapping) so that any waiter that read the old
    /// value will not sleep. `Release` publishes everything before it to a
    /// waiter that later reads the new value with `Acquire`.
    #[inline]
    pub(crate) fn bump(&self) {
        self.word.fetch_add(1, Ordering::Release);
    }

    /// Sleeps while the word equals `expected`, for at most `timeout`.
    /// May return early for any reason.
    pub(crate) fn wait(&self, expected: u32, timeout: Option<Duration>) {
        #[cfg(not(loom))]
        imp::wait(&self.word, expected, timeout);
        #[cfg(loom)]
        {
            // Loom does not model time; a timed wait is an untimed wait.
            let _ = timeout;
            let guard = self
                .lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.word.load(Ordering::Relaxed) == expected {
                drop(
                    self.cv
                        .wait(guard)
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
            }
        }
    }

    /// Wakes at most one thread sleeping on the word.
    pub(crate) fn wake_one(&self) {
        #[cfg(not(loom))]
        imp::wake(&self.word, false);
        #[cfg(loom)]
        {
            let _guard = self
                .lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.cv.notify_one();
        }
    }

    /// Wakes every thread sleeping on the word.
    pub(crate) fn wake_all(&self) {
        #[cfg(not(loom))]
        imp::wake(&self.word, true);
        #[cfg(loom)]
        {
            let _guard = self
                .lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.cv.notify_all();
        }
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use crate::sync::Arc;
    use std::thread;
    use std::time::Instant;

    #[test]
    fn mismatched_value_returns_immediately() {
        let f = Futex::new();
        let start = Instant::now();
        f.wait(1, None);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn timeout_is_respected() {
        let f = Futex::new();
        let start = Instant::now();
        f.wait(0, Some(Duration::from_millis(20)));
        let elapsed = start.elapsed();
        // Spurious early returns are allowed, but a timed wait must return.
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }

    #[test]
    fn wake_without_waiters_is_harmless() {
        let f = Futex::new();
        f.wake_one();
        f.wake_all();
    }

    fn wait_for_change(f: &Futex) {
        while f.load(Ordering::Acquire) == 0 {
            f.wait(0, None);
        }
    }

    #[test]
    fn bump_and_wake_one_releases_a_sleeper() {
        let f = Arc::new(Futex::new());
        let sleeper = {
            let f = Arc::clone(&f);
            thread::spawn(move || wait_for_change(&f))
        };
        thread::sleep(Duration::from_millis(20));
        f.bump();
        f.wake_one();
        sleeper.join().unwrap();
    }

    #[test]
    fn wake_all_releases_every_sleeper() {
        let f = Arc::new(Futex::new());
        let sleepers: Vec<_> = (0..4)
            .map(|_| {
                let f = Arc::clone(&f);
                thread::spawn(move || wait_for_change(&f))
            })
            .collect();
        thread::sleep(Duration::from_millis(20));
        f.bump();
        f.wake_all();
        for s in sleepers {
            s.join().unwrap();
        }
    }
}
