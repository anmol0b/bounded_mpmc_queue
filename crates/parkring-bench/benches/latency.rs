//! Wake latency versus idle CPU: the trade-off spin-then-park makes.
//!
//! A consumer waits on an empty queue. Every `GAP` the producer pushes one
//! item stamped with `Instant::now()`, and the consumer records how long it
//! took to receive it. The gap is long enough for our queues to park, so this
//! measures the cost of waking a parked thread, while process CPU time over
//! the run shows what each strategy burns while idle.
//!
//! Custom harness rather than criterion: every sample needs an idle gap, and
//! the interesting numbers are percentiles and CPU, not a mean.
//!
//! ```text
//! cargo bench -p parkring-bench --bench latency
//! ```
//! Writes `target/latency/latency.json` for `cargo run -p parkring-bench --example plot`.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

mod common;

use std::sync::Arc;
use std::sync::mpsc::channel;
use std::thread;
use std::time::{Duration, Instant};

use common::{BenchQueue, Crossbeam, StdChannel};
use parkring::{BlockingQueue, LockFreeQueue, ScqQueue};

const SAMPLES: usize = 2000;
const GAP: Duration = Duration::from_millis(2);

/// Process CPU time (user + system). Unix only; elsewhere the idle-CPU
/// column reads 0 and only latency is meaningful.
#[cfg(unix)]
fn cpu_time() -> Duration {
    // SAFETY: `getrusage` only writes into the zeroed struct we pass it.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, &raw mut usage), 0);
        usage
    };
    let tv = |t: libc::timeval| {
        Duration::from_secs(u64::try_from(t.tv_sec).unwrap_or(0))
            + Duration::from_micros(u64::try_from(t.tv_usec).unwrap_or(0))
    };
    tv(usage.ru_utime) + tv(usage.ru_stime)
}

#[cfg(not(unix))]
fn cpu_time() -> Duration {
    Duration::ZERO
}

struct Report {
    name: &'static str,
    p50_us: f64,
    p90_us: f64,
    p99_us: f64,
    idle_cpu_pct: f64,
}

fn measure<Q: BenchQueue>() -> Report {
    let queue = Arc::new(Q::with_capacity(64));
    let (tx, rx) = channel::<Instant>();
    let consumer = {
        let queue = Arc::clone(&queue);
        thread::spawn(move || {
            for _ in 0..SAMPLES {
                queue.pop();
                tx.send(Instant::now()).unwrap();
            }
        })
    };
    let mut latencies = Vec::with_capacity(SAMPLES);
    let (wall0, cpu0) = (Instant::now(), cpu_time());
    for i in 0..SAMPLES {
        thread::sleep(GAP);
        let sent = Instant::now();
        queue.push(i as u64);
        latencies.push(rx.recv().unwrap() - sent);
    }
    let (wall, cpu) = (wall0.elapsed(), cpu_time().saturating_sub(cpu0));
    consumer.join().unwrap();

    latencies.sort_unstable();
    let pct = |p: f64| {
        let idx = ((latencies.len() - 1) as f64 * p).round() as usize;
        latencies[idx].as_secs_f64() * 1e6
    };
    Report {
        name: Q::NAME,
        p50_us: pct(0.50),
        p90_us: pct(0.90),
        p99_us: pct(0.99),
        idle_cpu_pct: cpu.as_secs_f64() / wall.as_secs_f64() * 100.0,
    }
}

fn main() {
    // `cargo bench` passes `--bench`; `cargo test --benches` passes nothing
    // useful, so run a tiny smoke version there.
    if !std::env::args().any(|a| a == "--bench") {
        println!("latency: skipped outside `cargo bench`");
        return;
    }
    let reports = [
        measure::<LockFreeQueue<u64>>(),
        measure::<ScqQueue<u64>>(),
        measure::<Crossbeam>(),
        measure::<BlockingQueue<u64>>(),
        measure::<StdChannel>(),
    ];
    println!("\n| queue | p50 wake (µs) | p90 (µs) | p99 (µs) | CPU while idle |");
    println!("|---|---|---|---|---|");
    for r in &reports {
        println!(
            "| {} | {:.1} | {:.1} | {:.1} | {:.1}% |",
            r.name, r.p50_us, r.p90_us, r.p99_us, r.idle_cpu_pct
        );
    }
    let json: Vec<String> = reports
        .iter()
        .map(|r| {
            format!(
                r#"{{"queue":"{}","p50_us":{:.3},"p90_us":{:.3},"p99_us":{:.3},"idle_cpu_pct":{:.3}}}"#,
                r.name, r.p50_us, r.p90_us, r.p99_us, r.idle_cpu_pct
            )
        })
        .collect();
    let dir = parkring_bench::target_dir().join("latency");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("latency.json"),
        format!("[{}]\n", json.join(",\n")),
    )
    .unwrap();
}
