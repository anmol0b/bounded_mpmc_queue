//! Bit layout of an SCQ ring entry, and position arithmetic.
//!
//! A ring of `R = 2n` entries holds indices into an `n`-cell data array. Each
//! entry is one word:
//!
//! ```text
//!  63 ................... order+2 | order+1 | order .. 0
//!  [          cycle             ] [ safe  ] [  index   ]
//! ```
//!
//! * `index` has `order + 1` bits: values `0..n` are data indices and the
//!   all-ones value is ⊥ (empty).
//! * `safe` is the paper's `IsSafe` bit.
//! * `cycle` is the lap the entry belongs to: position `p` has cycle `p / R`.
//!
//! Positions are monotonic 63-bit counters, so cycles only grow and plain
//! integer comparison is correct (no modular arithmetic, no ABA).

/// Entries per cache line is `2^REMAP_BITS`: 128-byte lines on x86-64 and
/// AArch64, 8-byte entries. Set to 0 to disable remapping (for ablation).
const REMAP_BITS: u32 = if cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
    4
} else {
    3
};

/// Sizes derived from the capacity `n = 2^order`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Geometry {
    order: u32,
}

impl Geometry {
    pub(super) const fn new(order: u32) -> Self {
        Self { order }
    }

    /// Data cells, `n`.
    pub(super) const fn n(self) -> usize {
        1 << self.order
    }

    /// Ring entries, `R = 2n`.
    pub(super) const fn ring_len(self) -> usize {
        2 << self.order
    }

    const fn index_bits(self) -> u32 {
        self.order + 1
    }

    /// The empty index ⊥: all index bits set. Never a valid data index.
    pub(super) const fn bot(self) -> usize {
        (1 << self.index_bits()) - 1
    }

    const fn safe_bit(self) -> usize {
        1 << self.index_bits()
    }

    const fn cycle_shift(self) -> u32 {
        self.index_bits() + 1
    }

    pub(super) const fn pack(self, cycle: usize, safe: bool, index: usize) -> usize {
        (cycle << self.cycle_shift()) | ((safe as usize) << self.index_bits()) | index
    }

    pub(super) const fn cycle(self, entry: usize) -> usize {
        entry >> self.cycle_shift()
    }

    pub(super) const fn is_safe(self, entry: usize) -> bool {
        entry & self.safe_bit() != 0
    }

    pub(super) const fn index(self, entry: usize) -> usize {
        entry & self.bot()
    }

    /// The lap position `pos` belongs to.
    pub(super) const fn cycle_of(self, pos: usize) -> usize {
        pos >> self.index_bits()
    }

    /// The ring slot for position `pos`.
    ///
    /// Consecutive positions go to different cache lines: the low bits of the
    /// offset are rotated so that threads which obtained adjacent positions
    /// from the same fetch-add do not false-share. The paper calls this
    /// `cache_remap`. It is a bijection on `0..R`, so cycles stay consistent.
    pub(super) const fn slot(self, pos: usize) -> usize {
        remap(pos & (self.ring_len() - 1), self.index_bits())
    }

    /// Remaps an initial free-list index so the first pushes also spread
    /// across data cache lines.
    pub(super) const fn initial_index(self, i: usize) -> usize {
        remap(i, self.order)
    }
}

/// Rotates the low `bits` bits of `i` left by `REMAP_BITS`.
const fn remap(i: usize, bits: u32) -> usize {
    if bits <= REMAP_BITS {
        return i;
    }
    let mask = (1 << bits) - 1;
    ((i >> (bits - REMAP_BITS)) | (i << REMAP_BITS)) & mask
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trips() {
        for order in 0..20 {
            let g = Geometry::new(order);
            for &(cycle, safe, index) in &[
                (0, true, 0),
                (1, false, g.n() - 1),
                (12_345, true, g.bot()),
                (usize::MAX >> g.cycle_shift(), false, 0),
            ] {
                let e = g.pack(cycle, safe, index);
                assert_eq!((g.cycle(e), g.is_safe(e), g.index(e)), (cycle, safe, index));
            }
        }
    }

    #[test]
    fn bot_is_never_a_data_index() {
        for order in 0..20 {
            let g = Geometry::new(order);
            assert!(g.bot() >= g.n());
        }
    }

    #[test]
    fn remap_is_a_bijection() {
        for bits in 0..=16 {
            let len = 1usize << bits;
            let mut seen = vec![false; len];
            for i in 0..len {
                let r = remap(i, bits);
                assert!(r < len && !seen[r], "bits={bits} i={i}");
                seen[r] = true;
            }
        }
    }

    #[test]
    fn consecutive_positions_land_on_different_lines() {
        let g = Geometry::new(10);
        let line = |slot: usize| slot >> REMAP_BITS;
        for p in 0..64 {
            assert_ne!(line(g.slot(p)), line(g.slot(p + 1)));
        }
    }

    #[test]
    fn cycles_fit_below_the_closed_bit() {
        // The largest tail position is below 2^63; its packed cycle must fit.
        for order in 0..32 {
            let g = Geometry::new(order);
            let pos = (1usize << 63) - 1;
            let e = g.pack(g.cycle_of(pos), true, g.bot());
            assert_eq!(g.cycle(e), g.cycle_of(pos));
        }
    }
}
