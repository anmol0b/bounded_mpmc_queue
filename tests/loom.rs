//! Exhaustive interleaving checks with [loom].
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test --release --test loom
//! ```
//!
//! Under `--cfg loom` every atomic, mutex, condvar and `UnsafeCell` in the
//! crate is loom's model-checked version, and `Backoff` completes after one
//! step, so blocked threads reach the parking path immediately. Loom explores
//! every interleaving (up to a preemption bound) and reports a data race, a
//! failed assertion, or a deadlock, meaning a thread parked forever, which is
//! exactly what a lost wakeup looks like.
//!
//! The lock-free queue's minimum capacity is 2, so `new(1)` below is a
//! two-slot ring.
//!
//! [loom]: https://docs.rs/loom
#![cfg(loom)]

use loom::sync::Arc;
use loom::thread;
use parkring::{BlockingQueue, LockFreeQueue, PopError, TryPopError, TryPushError};

fn model(f: impl Fn() + Sync + Send + 'static) {
    model_with_bound(3, f);
}

/// Runs `f` under loom with at most `max_preemptions`.
///
/// `LOOM_MAX_PREEMPTIONS` can lower the bound but never raise it past the
/// test's own cap: some models are only finite under a small bound (see
/// `notify_one_does_not_strand_a_second_waiter`).
fn model_with_bound(max_preemptions: usize, f: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    // Three-thread scenarios with retry loops need longer execution paths
    // than loom's default budget of 1000 branches.
    builder.max_branches = std::env::var("LOOM_MAX_BRANCHES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let from_env = std::env::var("LOOM_MAX_PREEMPTIONS")
        .ok()
        .and_then(|v| v.parse().ok());
    builder.preemption_bound =
        Some(from_env.map_or(max_preemptions, |n: usize| n.min(max_preemptions)));
    builder.check(f);
}

/// L1. A consumer that finds the queue empty and parks must be woken by a
/// push that races with it. This is the classic lost-wakeup window.
#[test]
fn consumer_is_not_lost_when_parking_races_a_push() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
        let consumer = {
            let q = q.clone();
            thread::spawn(move || q.pop())
        };
        q.push(7).unwrap();
        assert_eq!(consumer.join().unwrap(), Ok(7));
    });
}

/// L2. The mirror image: a producer parked on a full queue must be woken by
/// a racing pop.
#[test]
fn producer_is_not_lost_when_parking_races_a_pop() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
        q.push(0).unwrap();
        q.push(1).unwrap();
        let producer = {
            let q = q.clone();
            thread::spawn(move || q.push(2))
        };
        assert_eq!(q.pop(), Ok(0));
        producer.join().unwrap().unwrap();
        assert_eq!(q.try_pop(), Ok(1));
        assert_eq!(q.try_pop(), Ok(2));
    });
}

/// L3. `close` must wake a consumer parked on an empty queue.
#[test]
fn close_wakes_a_parked_consumer() {
    model(|| {
        let q = Arc::new(LockFreeQueue::<u32>::new(2));
        let consumer = {
            let q = q.clone();
            thread::spawn(move || q.pop())
        };
        q.close();
        assert_eq!(consumer.join().unwrap(), Err(PopError));
    });
}

/// L8. `close` must wake a producer parked on a full queue and hand the item
/// back.
#[test]
fn close_wakes_a_parked_producer() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
        q.push(0).unwrap();
        q.push(1).unwrap();
        let producer = {
            let q = q.clone();
            thread::spawn(move || q.push(2))
        };
        q.close();
        assert_eq!(producer.join().unwrap().unwrap_err().into_inner(), 2);
    });
}

/// L4. Items pushed before `close` are still delivered, then `pop` fails.
#[test]
fn items_pushed_before_close_are_drained() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
        let consumer = {
            let q = q.clone();
            thread::spawn(move || (q.pop(), q.pop()))
        };
        q.push(1).unwrap();
        q.close();
        assert_eq!(consumer.join().unwrap(), (Ok(1), Err(PopError)));
    });
}

/// L5. Close is linearizable against push: a push that succeeds is always
/// delivered, and a consumer never sees "closed and empty" while a
/// successful push's item is still in flight.
#[test]
fn close_and_push_are_linearizable() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
        let producer = {
            let q = q.clone();
            thread::spawn(move || q.try_push(1))
        };
        q.close();
        let popped = q.try_pop();
        let pushed = producer.join().unwrap();
        match pushed {
            Ok(()) => {
                assert_ne!(popped, Err(TryPopError::Closed), "item stranded");
                let later = q.try_pop();
                assert!(
                    popped == Ok(1) || later == Ok(1),
                    "successful push was never delivered"
                );
            }
            Err(e) => {
                assert_eq!(e, TryPushError::Closed(1));
                assert_eq!(popped, Err(TryPopError::Closed));
            }
        }
    });
}

/// L6. Two parked consumers and two pushes: each push's `notify_one` must
/// reach a different consumer, so neither is stranded.
///
/// Run with a preemption bound of 1. With three threads and bound 2 or more,
/// loom can build unbounded schedules in which the two consumers yield back
/// and forth while the producer, preempted between claiming a slot and
/// publishing it, is never rescheduled. That is the documented non-lock-free
/// window (docs/DESIGN.md, section 7), not a bug a fair scheduler can hit.
/// Removing the wait on the unpublished slot made the model finite, which
/// confirmed the diagnosis. Blocking and wakeups are free context switches
/// in loom, so bound 1 still explores every park/notify ordering.
#[test]
fn notify_one_does_not_strand_a_second_waiter() {
    model_with_bound(1, || {
        let q = Arc::new(LockFreeQueue::new(2));
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

/// L7. Regression for the original bug: two concurrent `try_push` calls into
/// a queue with room for both must both succeed. The original code returned
/// `Err` from whichever thread lost the tail CAS.
#[test]
fn concurrent_try_push_never_fails_spuriously() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
        let other = {
            let q = q.clone();
            thread::spawn(move || q.try_push(1))
        };
        assert_eq!(q.try_push(2), Ok(()));
        assert_eq!(other.join().unwrap(), Ok(()));
        assert_eq!(q.len(), 2);
    });
}

/// L7 mirror: two concurrent `try_pop` calls on a queue holding two items
/// must both succeed, with distinct items.
#[test]
fn concurrent_try_pop_never_fails_spuriously() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
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

/// Items survive two full laps of a two-slot ring in FIFO order, exercising
/// the recycle store (`sequence = pos + capacity`).
#[test]
fn fifo_across_laps() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
        let producer = {
            let q = q.clone();
            thread::spawn(move || {
                for i in 0..4 {
                    q.push(i).unwrap();
                }
            })
        };
        for i in 0..4 {
            assert_eq!(q.pop(), Ok(i));
        }
        producer.join().unwrap();
    });
}

/// Items left in the queue are dropped exactly once when the last `Arc`
/// goes away on another thread. Loom's `UnsafeCell` tracking flags any
/// unsynchronised access.
#[test]
fn drop_on_another_thread_sees_published_items() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
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

/// Two consumers park; one push wakes one of them; `close` must still wake
/// the other. With the futex parker this exercises `notify_all` racing a
/// thread that was woken by `notify_one` but has not re-checked yet.
#[test]
fn close_after_one_of_two_parked_consumers_is_served() {
    model_with_bound(1, || {
        let q = Arc::new(LockFreeQueue::new(2));
        let consumers: Vec<_> = (0..2)
            .map(|_| {
                let q = q.clone();
                thread::spawn(move || q.pop())
            })
            .collect();
        q.push(1).unwrap();
        q.close();
        let got: Vec<_> = consumers.into_iter().map(|c| c.join().unwrap()).collect();
        assert!(
            got == [Ok(1), Err(PopError)] || got == [Err(PopError), Ok(1)],
            "{got:?}"
        );
    });
}

/// A timed pop with a deadline far in the future racing a push. Loom does not
/// model time, so this checks that the timed path's re-check and parking are
/// as sound as the untimed one.
#[test]
fn timed_pop_racing_a_push() {
    model(|| {
        let q = Arc::new(LockFreeQueue::new(2));
        let consumer = {
            let q = q.clone();
            thread::spawn(move || q.pop_timeout(std::time::Duration::from_secs(3600)))
        };
        q.push(9).unwrap();
        assert_eq!(consumer.join().unwrap(), Ok(9));
    });
}

/// The blocking queue goes through the same shim, so loom checks it too.
#[test]
fn blocking_queue_close_wakes_consumer_and_drains() {
    model(|| {
        let q = Arc::new(BlockingQueue::new(1));
        let consumer = {
            let q = q.clone();
            thread::spawn(move || (q.pop(), q.pop()))
        };
        q.push(1).unwrap();
        q.close();
        assert_eq!(consumer.join().unwrap(), (Ok(1), Err(PopError)));
    });
}
