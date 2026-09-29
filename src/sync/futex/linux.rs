//! `futex(2)` on Linux and Android.

use std::io;
use std::ptr;
use std::time::Duration;

use crate::sync::AtomicU32;

// `SYS_futex` is `c_int` on a few targets and `tv_nsec` is 32-bit on 32-bit
// targets, so the casts and fallible conversions are needed somewhere.
#[allow(clippy::unnecessary_cast, clippy::unnecessary_fallible_conversions)]
pub(super) fn wait(word: &AtomicU32, expected: u32, timeout: Option<Duration>) {
    // A relative timeout is fine: callers recompute it from a deadline on
    // every iteration, so EINTR cannot make the total wait drift.
    let ts = timeout.map(|d| libc::timespec {
        tv_sec: d.as_secs().try_into().unwrap_or(libc::time_t::MAX),
        tv_nsec: d.subsec_nanos().try_into().unwrap_or(0),
    });
    let ts_ptr = ts.as_ref().map_or(ptr::null(), ptr::from_ref);
    // SAFETY: `word` is a live, 4-byte-aligned atomic for the whole call
    // (borrowed from the caller), and FUTEX_WAIT only reads it atomically.
    // `ts_ptr` is null or points to a timespec on this stack frame.
    let r = unsafe {
        libc::syscall(
            libc::SYS_futex as libc::c_long,
            word.as_ptr(),
            libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
            expected,
            ts_ptr,
        )
    };
    if r < 0 {
        let err = io::Error::last_os_error().raw_os_error();
        debug_assert!(
            matches!(err, Some(libc::EAGAIN | libc::EINTR | libc::ETIMEDOUT)),
            "unexpected futex wait error: {err:?}"
        );
    }
}

#[allow(clippy::unnecessary_cast)]
pub(super) fn wake(word: &AtomicU32, all: bool) {
    let count: libc::c_int = if all { libc::c_int::MAX } else { 1 };
    // SAFETY: as above; FUTEX_WAKE does not dereference a private futex word.
    unsafe {
        libc::syscall(
            libc::SYS_futex as libc::c_long,
            word.as_ptr(),
            libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
            count,
        );
    }
}
