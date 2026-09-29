//! Loom models over raw tokens: an element is just a number carried in the
//! pointer, so a double take shows up as a failed assertion instead of a
//! double free.
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test -p parkring --release --lib deque
//! ```
//!
//! With `--cfg parkring_mutant="deque_no_pop_fence"` (or `no_steal_fence`)
//! `pop_racing_two_steals_never_double_takes` must fail; CI checks that.

use std::ptr::{self, NonNull};

use super::{Element, RawWorker, Steal};
use loom::thread;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Token(usize);

// SAFETY: the pointer is never dereferenced; it only carries the number.
unsafe impl Element for Token {
    fn into_raw(self) -> NonNull<()> {
        NonNull::new(ptr::without_provenance_mut(self.0 + 1)).expect("non-null")
    }
    unsafe fn from_raw(ptr: NonNull<()>) -> Self {
        Token(ptr.as_ptr().addr() - 1)
    }
}

fn model(max_preemptions: usize, f: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    builder.max_branches = 20_000;
    let from_env = std::env::var("LOOM_MAX_PREEMPTIONS")
        .ok()
        .and_then(|v| v.parse().ok());
    builder.preemption_bound =
        Some(from_env.map_or(max_preemptions, |n: usize| n.min(max_preemptions)));
    builder.check(f);
}

fn steal_until_settled(stealer: &super::RawStealer<Token>, attempts: usize) -> Vec<usize> {
    let mut taken = Vec::new();
    for _ in 0..attempts {
        loop {
            match stealer.steal() {
                Steal::Success(Token(v)) => {
                    taken.push(v);
                    break;
                }
                Steal::Empty => break,
                Steal::Retry => thread::yield_now(),
            }
        }
    }
    taken
}

fn assert_exactly(mut taken: Vec<usize>, expected: &[usize]) {
    taken.sort_unstable();
    let mut dedup = taken.clone();
    dedup.dedup();
    assert_eq!(taken, dedup, "an element was taken twice: {taken:?}");
    assert_eq!(taken, expected, "elements lost or invented");
}

/// The classic weak-memory failure (docs/DEQUE.md §5): with two elements, the
/// owner pops index 1 without a CAS while a thief steals index 0 and then
/// index 1. The pop fence makes the thief see the owner's reservation.
#[test]
fn pop_racing_two_steals_never_double_takes() {
    model(2, || {
        let worker = RawWorker::<Token>::new();
        worker.push(Token(0));
        worker.push(Token(1));
        let stealer = worker.stealer();
        let thief = thread::spawn(move || steal_until_settled(&stealer, 2));
        let mut taken: Vec<usize> = worker.pop().into_iter().map(|t| t.0).collect();
        taken.extend(thief.join().unwrap());
        while let Some(Token(v)) = worker.pop() {
            taken.push(v);
        }
        assert_exactly(taken, &[0, 1]);
    });
}

/// The last element: exactly one of the owner's pop and the thief's steal
/// gets it.
#[test]
fn last_element_goes_to_exactly_one_side() {
    model(3, || {
        let worker = RawWorker::<Token>::new();
        worker.push(Token(7));
        let stealer = worker.stealer();
        let thief = thread::spawn(move || steal_until_settled(&stealer, 1));
        let mut taken: Vec<usize> = worker.pop().into_iter().map(|t| t.0).collect();
        taken.extend(thief.join().unwrap());
        assert_exactly(taken, &[7]);
    });
}

/// Growth while a thief is stealing: the thief may read the old buffer or the
/// new one; either way every element is taken exactly once.
#[test]
fn growth_concurrent_with_steal() {
    model(2, || {
        let worker = RawWorker::<Token>::with_capacity_and_start(1, 0);
        worker.push(Token(0));
        let stealer = worker.stealer();
        let thief = thread::spawn(move || steal_until_settled(&stealer, 2));
        worker.push(Token(1));
        worker.push(Token(2));
        let mut taken = thief.join().unwrap();
        while let Some(Token(v)) = worker.pop() {
            taken.push(v);
        }
        assert_exactly(taken, &[0, 1, 2]);
    });
}

/// Indices that wrap: the owner overwrites a slot a slow thief may still be
/// reading. Atomic slots make that a harmless stale read; the thief's CAS
/// fails and it never uses what it read.
#[test]
fn slot_reuse_across_laps() {
    model(2, || {
        let worker = RawWorker::<Token>::with_capacity_and_start(2, usize::MAX - 1);
        worker.push(Token(0));
        worker.push(Token(1));
        let stealer = worker.stealer();
        let thief = thread::spawn(move || steal_until_settled(&stealer, 1));
        let mut taken: Vec<usize> = worker.pop().into_iter().map(|t| t.0).collect();
        worker.push(Token(2));
        taken.extend(thief.join().unwrap());
        while let Some(Token(v)) = worker.pop() {
            taken.push(v);
        }
        assert_exactly(taken, &[0, 1, 2]);
    });
}

/// Two thieves and the owner over two elements.
#[test]
fn two_thieves_and_the_owner() {
    model(1, || {
        let worker = RawWorker::<Token>::new();
        worker.push(Token(0));
        worker.push(Token(1));
        let thieves: Vec<_> = (0..2)
            .map(|_| {
                let stealer = worker.stealer();
                thread::spawn(move || steal_until_settled(&stealer, 1))
            })
            .collect();
        let mut taken: Vec<usize> = worker.pop().into_iter().map(|t| t.0).collect();
        for t in thieves {
            taken.extend(t.join().unwrap());
        }
        while let Some(Token(v)) = worker.pop() {
            taken.push(v);
        }
        assert_exactly(taken, &[0, 1]);
    });
}

/// The last handle is a stealer dropped on another thread; the deque's memory
/// (current and retired buffers) is freed there without a race.
#[test]
fn stealer_outlives_worker() {
    model(3, || {
        let worker = RawWorker::<Token>::with_capacity_and_start(1, 0);
        worker.push(Token(0));
        worker.push(Token(1)); // forces growth: one retired buffer
        let stealer = worker.stealer();
        let thief = thread::spawn(move || steal_until_settled(&stealer, 1));
        drop(worker);
        let taken = thief.join().unwrap();
        assert!(taken.len() <= 1);
    });
}
