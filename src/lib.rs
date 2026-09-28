//! Bounded multi-producer multi-consumer queues, in std-only Rust.
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
//! use bounded_mpmc_queue::LockFreeQueue;
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
//! assert_sync::<bounded_mpmc_queue::LockFreeQueue<std::rc::Rc<()>>>();
//! ```
//!
//! ```compile_fail
//! fn assert_sync<T: Sync>() {}
//! assert_sync::<bounded_mpmc_queue::BlockingQueue<std::rc::Rc<()>>>();
//! ```
//!
//! See `docs/DESIGN.md` in the repository for the memory-ordering argument
//! and how it is verified with loom and Miri.

mod error;
mod queue;
mod sync;
mod traits;
mod utils;

pub use error::{
    PopError, PopTimeoutError, PushError, PushTimeoutError, TryPopError, TryPushError,
};
pub use queue::{BlockingQueue, LockFreeQueue};
pub use sync::Backoff;
pub use traits::BoundedQueue;
