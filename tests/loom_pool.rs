//! Loom models for the work-stealing pool. Tiny pools (1-2 workers), since
//! each worker is a thread loom must interleave.
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test -p parkring --release --test loom_pool
//! ```
#![cfg(loom)]

use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use parkring::{ThreadPool, join};

fn model(max_preemptions: usize, f: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    builder.max_branches = 50_000;
    let from_env = std::env::var("LOOM_MAX_PREEMPTIONS")
        .ok()
        .and_then(|v| v.parse().ok());
    builder.preemption_bound =
        Some(from_env.map_or(max_preemptions, |n: usize| n.min(max_preemptions)));
    builder.check(f);
}

/// A job injected while the only worker may be going to sleep is not lost:
/// `install` returns.
#[test]
fn install_wakes_a_sleeping_worker() {
    model(2, || {
        let pool = ThreadPool::new(1);
        assert_eq!(pool.install(|| 7), 7);
    });
}

/// A spawned job runs exactly once, and dropping the pool waits for it.
#[test]
fn spawn_then_drop_runs_the_job_once() {
    model(2, || {
        let count = Arc::new(AtomicUsize::new(0));
        let pool = ThreadPool::new(1);
        let c = Arc::clone(&count);
        pool.spawn(move || {
            c.fetch_add(1, SeqCst);
        });
        drop(pool);
        assert_eq!(count.load(SeqCst), 1);
    });
}

/// With two workers, `join`'s second half may be stolen. Either way both
/// halves run exactly once and the results come back.
#[test]
fn join_with_a_possible_steal() {
    model(1, || {
        let pool = ThreadPool::new(2);
        let (a, b) = pool.install(|| join(|| 1, || 2));
        assert_eq!((a, b), (1, 2));
    });
}
