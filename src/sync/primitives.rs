//! The single source of every synchronisation primitive the crate uses.
//!
//! Under `RUSTFLAGS="--cfg loom"` these resolve to [loom]'s model-checked
//! versions, so the queue code is written once and verified against every
//! legal interleaving of the C11 memory model. CI rejects any use of the std
//! primitives outside this file, so nothing can bypass the shim.
//!
//! [loom]: https://docs.rs/loom

#[cfg(loom)]
pub(crate) use loom::{
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
    sync::{Condvar, Mutex, MutexGuard},
    thread,
};

#[cfg(not(loom))]
pub(crate) use std::{
    sync::atomic::{AtomicUsize, Ordering},
    sync::{Condvar, Mutex, MutexGuard},
    thread,
};

/// `std::cell::UnsafeCell` with loom's closure-based API.
///
/// Loom tracks every access made through `with`/`with_mut` and reports a
/// data race if two threads touch the same cell without a happens-before
/// edge. Mirroring that API on the std build keeps the queue source identical
/// in both configurations; the wrapper compiles to a plain pointer access.
#[cfg(not(loom))]
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct UnsafeCell<T>(std::cell::UnsafeCell<T>);

#[cfg(not(loom))]
impl<T> UnsafeCell<T> {
    #[inline]
    pub(crate) const fn new(value: T) -> Self {
        Self(std::cell::UnsafeCell::new(value))
    }

    #[inline]
    pub(crate) fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        f(self.0.get())
    }
}
