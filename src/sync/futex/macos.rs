//! `__ulock_wait` / `__ulock_wake` on macOS.
//!
//! Declared in xnu's `bsd/sys/ulock.h` and exported by libSystem. Private, but
//! ABI-stable since macOS 10.12 and used by libc++ for `std::atomic::wait`.

use std::ffi::{c_int, c_void};
use std::time::Duration;

use crate::sync::AtomicU32;

unsafe extern "C" {
    fn __ulock_wait(operation: u32, addr: *mut c_void, value: u64, timeout_us: u32) -> c_int;
    fn __ulock_wake(operation: u32, addr: *mut c_void, wake_value: u64) -> c_int;
}

const UL_COMPARE_AND_WAIT: u32 = 1;
const ULF_WAKE_ALL: u32 = 0x0000_0100;
/// Return `-errno` instead of setting `errno`.
const ULF_NO_ERRNO: u32 = 0x0100_0000;

pub(super) fn wait(word: &AtomicU32, expected: u32, timeout: Option<Duration>) {
    // 0 means "forever" to ulock, so a finite timeout is at least 1 µs. An
    // over-long timeout is clamped; the caller loops and re-waits.
    let timeout_us = timeout.map_or(0, |d| u32::try_from(d.as_micros()).unwrap_or(u32::MAX).max(1));
    // SAFETY: `word` is a live, aligned atomic borrowed for the call. With
    // UL_COMPARE_AND_WAIT the kernel atomically reads its 4 bytes and compares
    // them with `expected`; nothing is written.
    let r = unsafe {
        __ulock_wait(
            UL_COMPARE_AND_WAIT | ULF_NO_ERRNO,
            word.as_ptr().cast(),
            u64::from(expected),
            timeout_us,
        )
    };
    // r >= 0: woken. EINTR, ETIMEDOUT and EFAULT are all treated as spurious,
    // as libc++ does; the caller re-checks.
    debug_assert!(
        r >= 0 || matches!(-r, libc::EINTR | libc::ETIMEDOUT | libc::EFAULT),
        "unexpected __ulock_wait result: {r}"
    );
}

pub(super) fn wake(word: &AtomicU32, all: bool) {
    let op = UL_COMPARE_AND_WAIT | ULF_NO_ERRNO | if all { ULF_WAKE_ALL } else { 0 };
    loop {
        // SAFETY: as above; wake does not dereference the address.
        let r = unsafe { __ulock_wake(op, word.as_ptr().cast(), 0) };
        if r == -libc::EINTR {
            continue;
        }
        // ENOENT means nobody was waiting, which is fine.
        debug_assert!(r >= 0 || r == -libc::ENOENT, "unexpected __ulock_wake result: {r}");
        return;
    }
}
