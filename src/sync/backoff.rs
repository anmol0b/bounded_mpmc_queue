//! Exponential backoff for spin loops.

use crate::sync::thread;

/// Spin steps: `2^0 + ... + 2^6 = 127` spin hints before yielding.
const SPIN_LIMIT: u32 = 6;
/// After spinning, `snooze` yields to the OS scheduler until this step.
const YIELD_LIMIT: u32 = 10;

/// Exponential backoff for retry loops, modelled on crossbeam's `Backoff`.
///
/// * [`spin`](Self::spin) is for short, bounded waits such as a lost CAS or a
///   peer that is two instructions away from publishing. It never yields.
/// * [`snooze`](Self::snooze) is for waiting on an external condition such as
///   "queue not empty". It spins, then yields, and once
///   [`is_completed`](Self::is_completed) returns `true` the caller should
///   block instead of continuing to burn CPU.
///
/// Under `--cfg loom` every step is a model-checked `yield_now` and the
/// backoff completes immediately, so loom explores the parking path directly.
#[derive(Debug, Default)]
pub struct Backoff {
    step: u32,
}

impl Backoff {
    /// Creates a fresh backoff.
    #[inline]
    pub const fn new() -> Self {
        Self { step: 0 }
    }

    /// Restarts the backoff from the shortest wait.
    #[inline]
    pub fn reset(&mut self) {
        self.step = 0;
    }

    /// Busy-waits for `2^step` spin hints (capped). Never yields.
    #[inline]
    pub fn spin(&mut self) {
        #[cfg(loom)]
        thread::yield_now();
        #[cfg(not(loom))]
        for _ in 0..1u32 << self.step.min(SPIN_LIMIT) {
            std::hint::spin_loop();
        }
        if self.step <= SPIN_LIMIT {
            self.step += 1;
        }
    }

    /// Spins while the step is small, then yields the thread to the OS.
    #[inline]
    pub fn snooze(&mut self) {
        #[cfg(loom)]
        thread::yield_now();
        #[cfg(not(loom))]
        if self.step <= SPIN_LIMIT {
            for _ in 0..1u32 << self.step {
                std::hint::spin_loop();
            }
        } else {
            thread::yield_now();
        }
        if self.step <= YIELD_LIMIT {
            self.step += 1;
        }
    }

    /// Returns `true` once spinning and yielding are exhausted and the caller
    /// should park the thread instead.
    #[inline]
    pub fn is_completed(&self) -> bool {
        if cfg!(loom) {
            self.step > 0
        } else {
            self.step > YIELD_LIMIT
        }
    }
}
