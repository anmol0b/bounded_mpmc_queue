//! Position arithmetic for the lock-free queue.
//!
//! Positions (`head`, the low bits of `tail`, and every slot sequence) live
//! in `[0, 2^(BITS-1))`. The top bit of `tail` is the closed flag, so every
//! increment masks it out and every comparison is a signed difference modulo
//! `2^(BITS-1)`. Because capacities are powers of two they divide the position
//! space, so `pos & mask` stays consistent when positions wrap.

/// Set in `tail` once the queue is closed. Never cleared.
pub(crate) const CLOSED_BIT: usize = 1 << (usize::BITS - 1);

/// Mask selecting the position bits of `tail`.
pub(crate) const POS_MASK: usize = !CLOSED_BIT;

/// Largest supported capacity. Keeps every in-flight difference far from the
/// sign boundary of [`pos_diff`].
pub(crate) const MAX_CAPACITY: usize = 1 << (usize::BITS - 3);

/// `pos + n` in position space.
#[inline]
pub(crate) const fn pos_add(pos: usize, n: usize) -> usize {
    pos.wrapping_add(n) & POS_MASK
}

/// Signed `a - b` in position space.
///
/// The difference is taken modulo `2^(BITS-1)` and bit `BITS-2` is sign
/// extended, so values that straddle the wrap still compare correctly.
#[inline]
pub(crate) const fn pos_diff(a: usize, b: usize) -> isize {
    (((a.wrapping_sub(b)) & POS_MASK) << 1) as isize >> 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_wraps_without_touching_closed_bit() {
        assert_eq!(pos_add(POS_MASK, 1), 0);
        assert_eq!(pos_add(POS_MASK - 1, 3), 1);
        assert_eq!(pos_add(5, 3) & CLOSED_BIT, 0);
    }

    #[test]
    fn diff_is_signed_across_the_wrap() {
        assert_eq!(pos_diff(5, 3), 2);
        assert_eq!(pos_diff(3, 5), -2);
        assert_eq!(pos_diff(1, POS_MASK), 2);
        assert_eq!(pos_diff(POS_MASK, 1), -2);
        assert_eq!(pos_diff(0, 0), 0);
    }

    #[test]
    fn diff_ignores_closed_bit_on_either_side() {
        assert_eq!(pos_diff(7 | CLOSED_BIT, 3), 4);
        assert_eq!(pos_diff(7, 3 | CLOSED_BIT), 4);
    }
}
