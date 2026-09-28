//! Cache-line padding to prevent false sharing.

use std::ops::{Deref, DerefMut};

/// Aligns `T` to its own cache line so that writes to neighbouring fields do
/// not invalidate it.
///
/// 128 bytes on x86-64 (the adjacent-line prefetcher pulls lines in pairs) and
/// on AArch64 (Apple Silicon uses 128-byte lines); 64 bytes elsewhere. The
/// alignment alone pads the size, so no explicit padding field is needed.
#[cfg_attr(
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64"
    ),
    repr(align(128))
)]
#[cfg_attr(
    not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64"
    )),
    repr(align(64))
)]
#[derive(Debug, Default)]
pub(crate) struct CachePadded<T> {
    value: T,
}

impl<T> CachePadded<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self { value }
    }
}

impl<T> Deref for CachePadded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for CachePadded<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}
