//! Work-stealing deque: a sequential model, exactly-once delivery under
//! concurrency, and drop accounting.
#![allow(clippy::cast_possible_truncation)] // small test counts

mod common;

use std::collections::VecDeque;
use std::sync::Barrier;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::thread;

use common::drop_counter::{DropStats, Tracked};
use common::scale;
use parkring::{Steal, Worker};
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Op {
    Push(u32),
    Pop,
    Steal,
    Len,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => any::<u32>().prop_map(Op::Push),
        2 => Just(Op::Pop),
        2 => Just(Op::Steal),
        1 => Just(Op::Len),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: if cfg!(miri) { 8 } else { 512 },
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Single-threaded, the deque is a `VecDeque`: push/pop at the back,
    /// steal from the front, and a steal never has to retry. Small initial
    /// capacities force frequent growth; start indices near `usize::MAX`
    /// force wraparound.
    #[test]
    fn matches_vecdeque(
        capacity in 1usize..=8,
        near_wrap in any::<bool>(),
        ops in prop::collection::vec(op(), 0..400),
    ) {
        let start = if near_wrap { usize::MAX - 5 } else { 0 };
        let worker = Worker::with_capacity_and_start(capacity, start);
        let stealer = worker.stealer();
        let mut model = VecDeque::new();
        for op in ops {
            match op {
                Op::Push(v) => {
                    worker.push(v);
                    model.push_back(v);
                }
                Op::Pop => prop_assert_eq!(worker.pop(), model.pop_back()),
                Op::Steal => {
                    let expected = model.pop_front().map_or(Steal::Empty, Steal::Success);
                    prop_assert_eq!(stealer.steal(), expected);
                }
                Op::Len => prop_assert_eq!(worker.len(), model.len()),
            }
        }
        while let Some(v) = model.pop_back() {
            prop_assert_eq!(worker.pop(), Some(v));
        }
        prop_assert_eq!(worker.pop(), None);
    }
}

/// The owner pushes and pops in a mixed pattern while `thieves` threads
/// steal. Every item is taken exactly once, and each thief sees items in
/// increasing order: `top` only moves forward and lower indices were pushed
/// earlier.
fn exactly_once(thieves: usize, items: usize) {
    let worker = Worker::with_capacity_and_start(2, 0);
    let owner_done = AtomicBool::new(false);
    let start = Barrier::new(thieves + 1);
    let (owner_log, thief_logs) = thread::scope(|s| {
        let handles: Vec<_> = (0..thieves)
            .map(|_| {
                let stealer = worker.stealer();
                let (owner_done, start) = (&owner_done, &start);
                s.spawn(move || {
                    start.wait();
                    let mut log = Vec::new();
                    loop {
                        match stealer.steal() {
                            Steal::Success(v) => log.push(v),
                            Steal::Retry => {}
                            Steal::Empty if owner_done.load(SeqCst) => break,
                            Steal::Empty => thread::yield_now(),
                        }
                    }
                    log
                })
            })
            .collect();
        start.wait();
        let mut owner_log = Vec::new();
        for i in 0..items {
            worker.push(i);
            if i % 4 == 3 {
                owner_log.extend(worker.pop());
            }
        }
        while let Some(v) = worker.pop() {
            owner_log.push(v);
        }
        owner_done.store(true, SeqCst);
        let logs: Vec<Vec<usize>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        (owner_log, logs)
    });

    let mut seen = vec![false; items];
    for &v in owner_log.iter().chain(thief_logs.iter().flatten()) {
        assert!(!seen[v], "item {v} taken twice");
        seen[v] = true;
    }
    assert!(seen.iter().all(|s| *s), "items lost");
    for log in &thief_logs {
        assert!(
            log.windows(2).all(|w| w[0] < w[1]),
            "a thief saw items out of order"
        );
    }
}

#[test]
fn exactly_once_one_thief() {
    exactly_once(1, scale(50_000));
}

#[test]
fn exactly_once_many_thieves() {
    for thieves in [2, 4, 8] {
        exactly_once(thieves, scale(50_000));
    }
}

#[test]
fn leftover_items_are_dropped_exactly_once() {
    let stats = DropStats::new();
    let worker = Worker::with_capacity_and_start(1, 0);
    let stealer = worker.stealer();
    for i in 0..100 {
        worker.push(Tracked::new(i, &stats)); // grows several times
    }
    drop(worker.pop());
    drop(stealer.steal());
    assert_eq!(stats.live(), 98);
    drop(worker);
    assert_eq!(stats.live(), 98, "a stealer keeps the deque alive");
    drop(stealer);
    assert_eq!(stats.live(), 0);
    assert_eq!(stats.dropped(), 100);
}

#[test]
fn concurrent_run_leaves_nothing_behind() {
    let stats = DropStats::new();
    let worker = Worker::new();
    thread::scope(|s| {
        for _ in 0..3 {
            let stealer = worker.stealer();
            s.spawn(move || {
                for _ in 0..scale(2000) {
                    drop(stealer.steal());
                }
            });
        }
        for i in 0..scale(5000) as u64 {
            worker.push(Tracked::new(i, &stats));
            if i % 3 == 0 {
                drop(worker.pop());
            }
        }
    });
    drop(worker);
    assert_eq!(stats.live(), 0);
}
