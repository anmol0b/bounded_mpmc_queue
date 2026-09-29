//! Parking for threads that ran out of spin budget.
//!
//! Two implementations with the same interface:
//!
//! * [`epoch`]: waiters sleep on a futex word that notifiers bump. Used on
//!   Linux, Android and macOS, and under loom (with a modelled futex).
//! * [`condvar`]: a `Mutex<()>` + `Condvar`. Used everywhere else, under Miri
//!   on macOS (Miri has no `__ulock` shim), and with
//!   `--cfg parkring_force_condvar` so CI keeps it verified.

crate::sync::cfg_futex! {
    mod epoch;
    pub(crate) use epoch::WaitQueue;
}

crate::sync::cfg_no_futex! {
    mod condvar;
    pub(crate) use condvar::WaitQueue;
}
