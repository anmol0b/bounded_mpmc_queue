//! Work-stealing pool: results, draining on drop, panics, and nesting.

mod common;

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::thread;

use common::scale;
use parkring::{ThreadPool, join};

fn fib(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let (a, b) = join(|| fib(n - 1), || fib(n - 2));
    a + b
}

fn fib_sequential(n: u64) -> u64 {
    if n < 2 {
        n
    } else {
        fib_sequential(n - 1) + fib_sequential(n - 2)
    }
}

#[test]
fn fib_matches_sequential_at_several_thread_counts() {
    let n = if cfg!(miri) { 8 } else { 22 };
    for threads in [1, 2, 8] {
        let pool = ThreadPool::new(threads);
        assert_eq!(
            pool.install(|| fib(n)),
            fib_sequential(n),
            "{threads} threads"
        );
    }
}

#[test]
fn join_outside_a_pool_runs_sequentially() {
    assert_eq!(fib(15), fib_sequential(15));
}

#[test]
fn drop_runs_every_spawned_job() {
    let count = Arc::new(AtomicUsize::new(0));
    let jobs = scale(10_000);
    {
        let pool = ThreadPool::new(4);
        for _ in 0..jobs {
            let count = Arc::clone(&count);
            pool.spawn(move || {
                count.fetch_add(1, SeqCst);
            });
        }
    }
    assert_eq!(count.load(SeqCst), jobs);
}

#[test]
fn jobs_spawned_by_jobs_also_run_before_drop_returns() {
    let count = Arc::new(AtomicUsize::new(0));
    {
        let pool = Arc::new(ThreadPool::new(3));
        for _ in 0..scale(200) {
            let (count, inner_pool) = (Arc::clone(&count), Arc::clone(&pool));
            pool.spawn(move || {
                for _ in 0..5 {
                    let count = Arc::clone(&count);
                    inner_pool.spawn(move || {
                        count.fetch_add(1, SeqCst);
                    });
                }
            });
        }
        // The pool holds itself through spawned jobs; wait for them to
        // finish so ours is the last reference, then drop.
        while Arc::strong_count(&pool) > 1 {
            thread::yield_now();
        }
    }
    assert_eq!(count.load(SeqCst), scale(200) * 5);
}

#[test]
fn deep_recursion_of_joins() {
    fn depth(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            join(|| depth(n - 1), || 1).0 + 1
        }
    }
    let pool = ThreadPool::new(4);
    // The recursion runs on the pool's workers, which have default-size
    // stacks; debug-build join frames are several times larger.
    let n = if cfg!(miri) {
        50
    } else if cfg!(debug_assertions) {
        200
    } else {
        2_000
    };
    assert_eq!(pool.install(|| depth(n)), n);
}

fn boom(side: &str) -> u32 {
    panic!("{side}")
}

#[test]
fn panic_in_either_side_of_join_propagates_after_both_finish() {
    let pool = ThreadPool::new(2);
    let finished = AtomicUsize::new(0);
    let r = panic::catch_unwind(AssertUnwindSafe(|| {
        pool.install(|| {
            join(
                || boom("left"),
                || {
                    finished.fetch_add(1, SeqCst);
                },
            )
        })
    }));
    assert!(r.is_err());
    assert_eq!(finished.load(SeqCst), 1, "the other side ran to completion");

    let r = panic::catch_unwind(AssertUnwindSafe(|| {
        pool.install(|| join(|| 1, || boom("right")))
    }));
    assert!(r.is_err());
    // The pool still works afterwards.
    assert_eq!(pool.install(|| fib(10)), 55);
}

#[test]
fn concurrent_install_from_outside_threads() {
    let pool = ThreadPool::new(4);
    thread::scope(|s| {
        for i in 0..4u64 {
            let pool = &pool;
            s.spawn(move || {
                for _ in 0..scale(20) {
                    assert_eq!(pool.install(|| fib(12) + i), fib_sequential(12) + i);
                }
            });
        }
    });
}

#[test]
fn install_from_inside_runs_inline() {
    let pool = ThreadPool::new(2);
    let outer = pool.install(|| {
        let id = thread::current().id();
        (id, pool.install(|| thread::current().id()))
    });
    assert_eq!(outer.0, outer.1);
}

#[test]
#[should_panic(expected = "at least one thread")]
fn zero_threads_panics() {
    let _ = ThreadPool::new(0);
}
