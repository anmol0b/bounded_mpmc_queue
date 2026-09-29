//! Loom models for `ScqQueue`.
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test -p parkring --release --test loom_scq
//! ```
//!
//! Capacities are 1 or 2, so rings have 2 or 4 entries and the threshold is 2
//! or 5: small enough for loom, and small enough to reach the threshold,
//! catchup and invalidation paths.
#![cfg(all(loom, target_pointer_width = "64"))]

use loom::sync::Arc;
use loom::thread;
use parkring::{PopError, ScqQueue, TryPopError, TryPushError};

fn model(f: impl Fn() + Sync + Send + 'static) {
    model_with_bound(3, f);
}

/// `LOOM_MAX_PREEMPTIONS` may lower a test's bound but never raise it.
fn model_with_bound(max_preemptions: usize, f: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    builder.max_branches = 20_000;
    let from_env = std::env::var("LOOM_MAX_PREEMPTIONS")
        .ok()
        .and_then(|v| v.parse().ok());
    builder.preemption_bound =
        Some(from_env.map_or(max_preemptions, |n: usize| n.min(max_preemptions)));
    builder.check(f);
}

/// S1. A consumer that parks on an empty queue is woken by a racing push.
/// The wake pairs on the allocated ring's threshold swap, not on `tail`.
#[test]
fn consumer_is_not_lost_when_parking_races_a_push() {
    model(|| {
        let q = Arc::new(ScqQueue::new(1));
        let consumer = {
            let q = q.clone();
            thread::spawn(move || q.pop())
        };
        q.push(7).unwrap();
        assert_eq!(consumer.join().unwrap(), Ok(7));
    });
}

/// S2. A producer parked on a full queue is woken by a racing pop, through
/// the free ring.
#[test]
fn producer_is_not_lost_when_parking_races_a_pop() {
    model(|| {
        let q = Arc::new(ScqQueue::new(1));
        q.push(0).unwrap();
        let producer = {
            let q = q.clone();
            thread::spawn(move || q.push(1))
        };
        assert_eq!(q.pop(), Ok(0));
        producer.join().unwrap().unwrap();
        assert_eq!(q.try_pop(), Ok(1));
    });
}

/// S3. `close` wakes a parked consumer and a parked producer.
#[test]
fn close_wakes_parked_threads() {
    model(|| {
        let empty = Arc::new(ScqQueue::<u32>::new(1));
        let full = Arc::new(ScqQueue::new(1));
        full.push(0).unwrap();
        let consumer = {
            let q = empty.clone();
            thread::spawn(move || q.pop())
        };
        let producer = {
            let q = full.clone();
            thread::spawn(move || q.push(1))
        };
        empty.close();
        full.close();
        assert_eq!(consumer.join().unwrap(), Err(PopError));
        assert_eq!(producer.join().unwrap().unwrap_err().into_inner(), 1);
    });
}

/// S4. Close is linearizable against push: a successful push is delivered,
/// and a rejected push leaves nothing behind.
#[test]
fn close_and_push_are_linearizable() {
    model(|| {
        let q = Arc::new(ScqQueue::new(2));
        let producer = {
            let q = q.clone();
            thread::spawn(move || q.try_push(1))
        };
        q.close();
        let popped = q.try_pop();
        match producer.join().unwrap() {
            Ok(()) => {
                assert_ne!(popped, Err(TryPopError::Closed), "item stranded");
                assert!(popped == Ok(1) || q.try_pop() == Ok(1));
            }
            Err(e) => {
                assert_eq!(e, TryPushError::Closed(1));
                assert_eq!(popped, Err(TryPopError::Closed));
                assert_eq!(q.try_pop(), Err(TryPopError::Closed));
            }
        }
    });
}

/// S5. With no pop in flight, concurrent `try_push` calls into a queue with
/// room for all of them succeed.
#[test]
fn concurrent_try_push_never_fails_without_a_pop_in_flight() {
    model(|| {
        let q = Arc::new(ScqQueue::new(2));
        let other = {
            let q = q.clone();
            thread::spawn(move || q.try_push(1))
        };
        assert_eq!(q.try_push(2), Ok(()));
        assert_eq!(other.join().unwrap(), Ok(()));
        assert_eq!(q.len(), 2);
    });
}

/// S5 mirror: concurrent `try_pop` calls on a queue holding enough items
/// both succeed, with distinct items.
#[test]
fn concurrent_try_pop_takes_distinct_items() {
    model(|| {
        let q = Arc::new(ScqQueue::new(2));
        q.push(1).unwrap();
        q.push(2).unwrap();
        let other = {
            let q = q.clone();
            thread::spawn(move || q.try_pop())
        };
        let mine = q.try_pop().unwrap();
        let theirs = other.join().unwrap().unwrap();
        assert_ne!(mine, theirs);
    });
}

/// S6. FIFO across several laps of a one-cell queue: indices cycle through
/// both rings and cycles advance.
#[test]
fn fifo_across_laps() {
    model(|| {
        let q = Arc::new(ScqQueue::new(1));
        let producer = {
            let q = q.clone();
            thread::spawn(move || {
                for i in 0..3 {
                    q.push(i).unwrap();
                }
            })
        };
        for i in 0..3 {
            assert_eq!(q.pop(), Ok(i));
        }
        producer.join().unwrap();
    });
}

/// S7. An empty `try_pop` racing a `try_push`: the pop may invalidate the
/// position the push claimed, forcing it to retry. Whatever happens, the item
/// is delivered exactly once.
#[test]
fn empty_pop_racing_push_loses_nothing() {
    model(|| {
        let q = Arc::new(ScqQueue::new(1));
        let consumer = {
            let q = q.clone();
            thread::spawn(move || q.try_pop())
        };
        q.try_push(5).unwrap();
        match consumer.join().unwrap() {
            Ok(v) => {
                assert_eq!(v, 5);
                assert_eq!(q.try_pop(), Err(TryPopError::Empty));
            }
            Err(TryPopError::Empty) => assert_eq!(q.try_pop(), Ok(5)),
            Err(e) => panic!("{e:?}"),
        }
    });
}

/// S8. Drive the threshold negative with empty pops, then race a push with a
/// blocking pop. A negative threshold must not strand the item.
#[test]
fn exhausted_threshold_does_not_strand_an_item() {
    model(|| {
        let q = Arc::new(ScqQueue::new(1));
        for _ in 0..4 {
            assert_eq!(q.try_pop(), Err(TryPopError::Empty));
        }
        let consumer = {
            let q = q.clone();
            thread::spawn(move || q.pop())
        };
        q.push(3).unwrap();
        assert_eq!(consumer.join().unwrap(), Ok(3));
    });
}

/// S10. Two parked consumers and two pushes: each `notify_one` must reach a
/// different consumer. The Vyukov queue's version of this model is only
/// finite with a preemption bound of 1, because a consumer can wait forever
/// on a preempted producer. SCQ never waits on a particular thread, so this
/// model is finite at bound 2.
#[test]
fn notify_one_does_not_strand_a_second_waiter() {
    model_with_bound(2, || {
        let q = Arc::new(ScqQueue::new(2));
        let consumers: Vec<_> = (0..2)
            .map(|_| {
                let q = q.clone();
                thread::spawn(move || q.pop().unwrap())
            })
            .collect();
        q.push(1).unwrap();
        q.push(2).unwrap();
        let mut got: Vec<_> = consumers.into_iter().map(|c| c.join().unwrap()).collect();
        got.sort_unstable();
        assert_eq!(got, vec![1, 2]);
    });
}

/// S11. The last reference is dropped on another thread; the remaining item
/// is dropped exactly once.
#[test]
fn drop_on_another_thread_releases_items() {
    model(|| {
        let q = Arc::new(ScqQueue::new(2));
        let item = std::sync::Arc::new(());
        let producer = {
            let (q, item) = (q.clone(), item.clone());
            thread::spawn(move || q.push(item).unwrap())
        };
        drop(q);
        producer.join().unwrap();
        assert_eq!(std::sync::Arc::strong_count(&item), 1);
    });
}
