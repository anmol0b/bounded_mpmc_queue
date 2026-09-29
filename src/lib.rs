//! Bounded multi-producer multi-consumer queues.
//!
//! The only dependency is `libc`, used on Linux, Android and macOS to park
//! threads on a futex. Other platforms use std's `Mutex` and `Condvar`.
//!
//! | | [`LockFreeQueue`] | [`BlockingQueue`] |
//! |---|---|---|
//! | Fast path | one CAS + one `Release` store, no lock | one mutex acquisition |
//! | Waiting | spin, yield, then park on a condvar | park on a condvar |
//! | Capacity | rounded up to a power of two, minimum 2 | exact |
//! | Scales with threads | yes | serialises on the mutex |
//!
//! Both implement [`BoundedQueue`] and share one API:
//!
//! * `push` / `pop` block (the lock-free queue parks after a short spin, so an
//!   idle thread does not burn a core);
//! * `try_push` / `try_pop` never block and never fail spuriously;
//! * `push_timeout` / `pop_timeout` give up after a deadline;
//! * `close` rejects further pushes, wakes every waiter, and lets consumers
//!   drain the remaining items before `pop` reports [`PopError`].
//!
//! Every failed push hands the item back inside the error.
//!
//! ```
//! use parkring::LockFreeQueue;
//!
//! let queue = LockFreeQueue::new(64);
//! std::thread::scope(|s| {
//!     let consumers: Vec<_> = (0..4)
//!         .map(|_| s.spawn(|| {
//!             let mut popped = 0;
//!             while queue.pop().is_ok() {
//!                 popped += 1;
//!             }
//!             popped
//!         }))
//!         .collect();
//!
//!     // Every producer is joined when this inner scope ends.
//!     std::thread::scope(|p| {
//!         for _ in 0..4 {
//!             p.spawn(|| (0..1000).for_each(|i| queue.push(i).unwrap()));
//!         }
//!     });
//!
//!     // Consumers drain what is left, then `pop` returns `Err` and they exit.
//!     queue.close();
//!     let total: usize = consumers.into_iter().map(|h| h.join().unwrap()).sum();
//!     assert_eq!(total, 4000);
//! });
//! ```
//!
//! # Thread safety
//!
//! Both queues are `Send + Sync` exactly when `T: Send`. Items move between
//! threads but are never shared, so `T: Sync` is not required:
//!
//! ```compile_fail
//! fn assert_sync<T: Sync>() {}
//! assert_sync::<parkring::LockFreeQueue<std::rc::Rc<()>>>();
//! ```
//!
//! ```compile_fail
//! fn assert_sync<T: Sync>() {}
//! assert_sync::<parkring::BlockingQueue<std::rc::Rc<()>>>();
//! ```
//!
//! ```compile_fail
//! fn assert_sync<T: Sync>() {}
//! assert_sync::<parkring::ScqQueue<std::rc::Rc<()>>>();
//! ```
//!
//! A deque's [`Worker`] belongs to one thread at a time: it is `Send` but not
//! `Sync`. [`Stealer`] is `Send + Sync`.
//!
//! ```compile_fail
//! fn assert_sync<T: Sync>() {}
//! assert_sync::<parkring::Worker<u32>>();
//! ```
//!
//! ```compile_fail
//! fn assert_send<T: Send>() {}
//! assert_send::<parkring::Stealer<std::rc::Rc<()>>>();
//! ```
//!
//! See `docs/DESIGN.md` in the repository for the memory-ordering argument
//! and how it is verified with loom and Miri.

mod deque;
mod error;
mod queue;
mod sync;
mod traits;
mod utils;

pub use deque::{Steal, Stealer, Worker};
pub use error::{
    PopError, PopTimeoutError, PushError, PushTimeoutError, TryPopError, TryPushError,
};
#[cfg(target_pointer_width = "64")]
pub use queue::ScqQueue;
pub use queue::{BlockingQueue, LockFreeQueue};
pub use sync::Backoff;
pub use traits::BoundedQueue;

/// Compiles and runs the README's code examples as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
