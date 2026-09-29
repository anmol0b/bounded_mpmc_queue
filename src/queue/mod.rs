//! Queue implementations.

mod blocking;
mod lockfree;
mod ring_buffer;
#[cfg(target_pointer_width = "64")]
mod scq;
mod slot;

pub use blocking::BlockingQueue;
pub use lockfree::LockFreeQueue;
#[cfg(target_pointer_width = "64")]
pub use scq::ScqQueue;
