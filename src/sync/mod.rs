//! Synchronisation building blocks.

/// Items compiled when parking uses a futex: Linux, Android, macOS (not
/// under Miri, which has no `__ulock` shim) and loom, unless
/// `--cfg parkring_force_condvar` selects the portable fallback.
macro_rules! cfg_futex {
    ($($item:item)*) => {$(
        #[cfg(all(
            not(parkring_force_condvar),
            any(
                loom,
                target_os = "linux",
                target_os = "android",
                all(target_os = "macos", not(miri)),
            ),
        ))]
        $item
    )*};
}

/// The complement of [`cfg_futex`].
macro_rules! cfg_no_futex {
    ($($item:item)*) => {$(
        #[cfg(not(all(
            not(parkring_force_condvar),
            any(
                loom,
                target_os = "linux",
                target_os = "android",
                all(target_os = "macos", not(miri)),
            ),
        )))]
        $item
    )*};
}
pub(crate) use {cfg_futex, cfg_no_futex};

mod backoff;
cfg_futex! {
    pub(crate) mod futex;
}
pub(crate) mod pos;
mod primitives;
mod wait_queue;

pub use backoff::Backoff;
pub(crate) use primitives::*;
pub(crate) use wait_queue::WaitQueue;
