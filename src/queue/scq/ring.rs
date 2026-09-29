//! One SCQ index ring (Nikolaev, "A Scalable, Portable, and Memory-Efficient
//! Lock-Free FIFO Queue", DISC 2019).
//!
//! The ring stores indices in `0..n` using `R = 2n` entries. Producers and
//! consumers claim positions with `fetch_add` on `tail` and `head`, so a
//! contended claim never retries; a CAS only arbitrates the single entry the
//! position maps to.
//!
//! See `docs/SCQ.md` for the algorithm, the correctness argument, and the two
//! places this implementation deliberately differs from the paper.

use super::entry::Geometry;
use crate::sync::pos::{CLOSED_BIT, POS_MASK};
use crate::sync::{
    AtomicIsize, AtomicUsize, Backoff,
    Ordering::{AcqRel, Acquire, Relaxed},
};
use crate::utils::CachePadded;

/// How many times a dequeuer re-reads an empty entry whose enqueuer has
/// already claimed the position (`tail > head`) before invalidating it. Not in
/// the paper; bounded, so the ring stays lock-free.
const SPIN_ATTEMPTS: usize = if cfg!(loom) { 1 } else { 64 };

/// Result of a ring dequeue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Deq {
    /// A dequeued data index.
    Item(usize),
    /// `head` caught up with `tail`: the ring was empty. `closed` is the
    /// closed flag read from `tail` at that moment.
    EmptyAtTail { closed: bool },
    /// The threshold ran out: the ring is empty (see `docs/SCQ.md`).
    EmptyByThreshold,
}

pub(super) struct IndexRing {
    head: CachePadded<AtomicUsize>,
    /// Enqueue position; the top bit is the closed flag (allocated ring only).
    tail: CachePadded<AtomicUsize>,
    /// The paper's `Threshold`. Reset to `3n - 1` by every successful
    /// enqueue, decremented by failed dequeue attempts; negative means empty.
    threshold: CachePadded<AtomicIsize>,
    entries: Box<[AtomicUsize]>,
    geo: Geometry,
}

impl IndexRing {
    /// A ring with no indices. `start` must be a positive multiple of `2n`.
    pub(super) fn new_empty(geo: Geometry, start: usize) -> Self {
        debug_assert!(start >= geo.ring_len() && start % geo.ring_len() == 0);
        let past = geo.cycle_of(start) - 1;
        let entries = (0..geo.ring_len())
            .map(|_| AtomicUsize::new(geo.pack(past, true, geo.bot())))
            .collect();
        Self {
            head: CachePadded::new(AtomicUsize::new(start)),
            tail: CachePadded::new(AtomicUsize::new(start)),
            threshold: CachePadded::new(AtomicIsize::new(-1)),
            entries,
            geo,
        }
    }

    /// A ring holding every index `0..n` once.
    pub(super) fn new_full(geo: Geometry, start: usize) -> Self {
        let ring = Self::new_empty(geo, start);
        let cycle = geo.cycle_of(start);
        for i in 0..geo.n() {
            let pos = start + i;
            ring.entries[geo.slot(pos)].store(geo.pack(cycle, true, geo.initial_index(i)), Relaxed);
        }
        ring.tail.store(start + geo.n(), Relaxed);
        ring.threshold.store(ring.threshold_max(), Relaxed);
        ring
    }

    fn threshold_max(&self) -> isize {
        (3 * self.geo.n() - 1) as isize
    }

    fn entry(&self, pos: usize) -> &AtomicUsize {
        &self.entries[self.geo.slot(pos)]
    }

    /// Enqueues `index`. Never full: at most `n` indices exist and the ring
    /// has `2n` entries. With `closable`, fails once `tail` carries the
    /// closed flag, which orders every enqueue against `close`.
    pub(super) fn enqueue(&self, index: usize, closable: bool) -> Result<(), ()> {
        let g = self.geo;
        loop {
            // E1: claim a position. Relaxed: no data flows through `tail`,
            // and an RMW always reads the latest value, including the flag.
            let raw = self.tail.fetch_add(1, Relaxed);
            if closable && raw & CLOSED_BIT != 0 {
                return Err(());
            }
            let t = raw & POS_MASK;
            let tc = g.cycle_of(t);
            let slot = self.entry(t);
            // E2: Acquire, so an "unsafe" mark (and the dequeuer's `head`
            // increment sequenced before it) is visible to the check below.
            let mut e = slot.load(Acquire);
            loop {
                if g.cycle(e) >= tc || g.index(e) != g.bot() {
                    break; // a later lap owns the entry, or it is occupied
                }
                // E3, the IsSafe rule: a dequeuer marked this entry unsafe
                // and may already have passed position `t`.
                if !g.is_safe(e) && self.head.load(Relaxed) > t {
                    break;
                }
                // E4: publish. Release makes the data written for this index
                // (or, in the free ring, the finished read of its cell)
                // visible to whoever dequeues it.
                match slot.compare_exchange_weak(e, g.pack(tc, true, index), AcqRel, Acquire) {
                    Ok(_) => {
                        // E5: reset the threshold. An unconditional swap, not
                        // the paper's load-then-store: a stale load could leave
                        // the threshold negative while this item is present.
                        // The swap is also the word parked threads re-check.
                        #[cfg(not(parkring_mutant = "scq_conditional_threshold"))]
                        self.threshold.swap(self.threshold_max(), AcqRel);
                        #[cfg(parkring_mutant = "scq_conditional_threshold")]
                        if self.threshold.load(Relaxed) != self.threshold_max() {
                            self.threshold.store(self.threshold_max(), Relaxed);
                        }
                        return Ok(());
                    }
                    Err(current) => e = current,
                }
            }
        }
    }

    /// Dequeues an index. `drain` ignores the threshold, so that after
    /// `close` consumers only stop once `head` has caught up with `tail`.
    pub(super) fn dequeue(&self, drain: bool) -> Deq {
        let g = self.geo;
        // D0: a negative threshold means the ring is probably empty. The
        // paper treats it as proof, but its 3n - 1 bound counts positions,
        // not dequeuers that claimed a position before the last reset and
        // decrement after it; their number is bounded by the thread count.
        // With more threads than capacity the threshold can go negative while
        // an item sits at `head` (found by the capacity-1, 3+3 thread test).
        // So we also require `tail <= head` before trusting it. A negative
        // threshold still limits every call to one iteration (D8), which is
        // what prevents the livelock the paper introduced it for.
        if !drain
            && self.threshold.load(Relaxed) < 0
            && self.tail.load(Relaxed) & POS_MASK <= self.head.load(Relaxed)
        {
            return Deq::EmptyByThreshold;
        }
        let mut backoff = Backoff::new();
        loop {
            // D1: claim a position. Made visible to enqueuers by D5's Release.
            let h = self.head.fetch_add(1, Relaxed);
            let hc = g.cycle_of(h);
            let slot = self.entry(h);
            // D2: Acquire synchronises with the enqueuer's E4.
            let mut e = slot.load(Acquire);
            let mut waited = 0;
            loop {
                if g.cycle(e) == hc {
                    debug_assert_ne!(g.index(e), g.bot(), "position {h} consumed twice");
                    // Consume: set the index to ⊥, keeping cycle and IsSafe.
                    // An OR rather than a store, so a concurrent unsafe mark
                    // is not lost.
                    slot.fetch_or(g.bot(), Relaxed);
                    return Deq::Item(g.index(e));
                }
                if g.cycle(e) > hc {
                    break; // a later lap already passed this entry
                }
                let new = if g.index(e) == g.bot() {
                    // Empty entry of an older lap. The enqueuer for `h` may
                    // be between its fetch-add and its CAS: wait briefly.
                    if waited < SPIN_ATTEMPTS && self.tail.load(Relaxed) & POS_MASK > h {
                        waited += 1;
                        backoff.spin();
                        e = slot.load(Acquire);
                        continue;
                    }
                    // Advance the entry to this lap, so that a late enqueuer
                    // for `h` fails its CAS and takes a new position.
                    g.pack(hc, g.is_safe(e), g.bot())
                } else {
                    // An older lap's item a slow consumer has not taken yet:
                    // mark it unsafe and leave it.
                    g.pack(g.cycle(e), false, g.index(e))
                };
                // D5: always a CAS, even when `new == e`. The Release on this
                // RMW is what publishes our `head` increment to an enqueuer
                // that later reads the entry; skipping it when the entry
                // already looks right (as the reference code does) removes
                // that edge. A failed CAS returns the current value, which we
                // re-examine. See docs/SCQ.md: this is argued, not
                // model-checked.
                match slot.compare_exchange_weak(e, new, AcqRel, Acquire) {
                    Ok(_) => break,
                    Err(current) => e = current,
                }
            }
            // No item at `h`.
            let tail = self.tail.load(Relaxed);
            if tail & POS_MASK <= h + 1 {
                self.catchup(tail, h + 1);
                self.threshold.fetch_sub(1, AcqRel);
                return Deq::EmptyAtTail {
                    closed: tail & CLOSED_BIT != 0,
                };
            }
            if !drain && self.threshold.fetch_sub(1, AcqRel) <= 0 {
                return Deq::EmptyByThreshold;
            }
        }
    }

    /// Moves `tail` up to `head` after dequeuers overran it, so positions stay
    /// in step. Preserves the closed flag.
    fn catchup(&self, mut tail: usize, mut head: usize) {
        loop {
            let new = head | (tail & CLOSED_BIT);
            match self.tail.compare_exchange_weak(tail, new, Relaxed, Relaxed) {
                Ok(_) => return,
                Err(current) => tail = current,
            }
            head = self.head.load(Relaxed);
            if tail & POS_MASK >= head {
                return;
            }
        }
    }

    /// Sets the closed flag. Returns `true` if this call set it.
    pub(super) fn close(&self) -> bool {
        self.tail.fetch_or(CLOSED_BIT, AcqRel) & CLOSED_BIT == 0
    }

    pub(super) fn is_closed(&self) -> bool {
        self.tail.load(Acquire) & CLOSED_BIT != 0
    }

    /// Wake condition for threads parked on this ring becoming non-empty.
    ///
    /// Re-checks through an RMW on `threshold`, the word every successful
    /// enqueue swaps after publishing. That pairs with the enqueuer's swap
    /// through the release sequence, exactly as the Vyukov queue pairs with
    /// its tail CAS, so no wakeup is lost. `tail` itself cannot be the pairing
    /// word here: its fetch-add happens before the item is published.
    ///
    /// The threshold's value is deliberately ignored (see D0): readiness is
    /// `tail > head`. Positions left empty by invalidated enqueues can make it
    /// true with nothing to take; the next `try_pop` sweeps them and catches
    /// `tail` up, so a waiter spins at most once per hole before parking.
    pub(super) fn ready(&self) -> bool {
        self.threshold.fetch_add(0, AcqRel);
        let tail = self.tail.load(Relaxed);
        tail & CLOSED_BIT != 0 || tail & POS_MASK > self.head.load(Relaxed)
    }

    /// A snapshot of the number of indices in the ring. Exact when
    /// quiescent; may over-count positions left empty by aborted enqueues.
    pub(super) fn len(&self) -> usize {
        loop {
            let tail = self.tail.load(Acquire);
            let head = self.head.load(Acquire);
            if self.tail.load(Acquire) == tail {
                return (tail & POS_MASK).saturating_sub(head).min(self.geo.n());
            }
        }
    }

    /// Calls `f` with every index still in the ring. Requires exclusive
    /// access (only used by `Drop`).
    pub(super) fn for_each_index(&mut self, mut f: impl FnMut(usize)) {
        let g = self.geo;
        for entry in &*self.entries {
            let index = g.index(entry.load(Relaxed));
            if index != g.bot() {
                f(index);
            }
        }
    }
}

impl std::fmt::Debug for IndexRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexRing")
            .field("head", &self.head.load(Relaxed))
            .field("tail", &(self.tail.load(Relaxed) & POS_MASK))
            .field("threshold", &self.threshold.load(Relaxed))
            .field(
                "entries",
                &self
                    .entries
                    .iter()
                    .map(|e| {
                        let e = e.load(Relaxed);
                        let g = self.geo;
                        (g.cycle(e), g.is_safe(e), g.index(e))
                    })
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}
