# bounded_mpmc_queue

[![CI](https://github.com/anmol0b/bounded_mpmc_queue/actions/workflows/ci.yml/badge.svg)](https://github.com/anmol0b/bounded_mpmc_queue/actions/workflows/ci.yml)
![MSRV 1.85](https://img.shields.io/badge/MSRV-1.85-blue)
![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

Bounded multi-producer multi-consumer queues in std-only Rust.

* **`LockFreeQueue`**: Dmitry Vyukov's per-slot sequence ring. The fast path is
  one CAS plus one `Release` store with no locks. Blocked threads spin briefly,
  then **park**, so a consumer waiting on an idle queue uses no CPU.
* **`BlockingQueue`**: one `Mutex` and two `Condvar`s, the obviously-correct
  reference implementation.

Both support blocking, non-blocking and timed operations, plus `close()` with
drain semantics. Both are verified with **loom** model checking, **Miri**,
property-based tests, and drop accounting.

```rust
use bounded_mpmc_queue::LockFreeQueue;

let queue = LockFreeQueue::new(1024);
std::thread::scope(|s| {
    let consumer = s.spawn(|| {
        let mut sum = 0u64;
        while let Ok(v) = queue.pop() {   // parks while empty
            sum += v;
        }
        sum                               // Err(PopError) once closed and drained
    });
    for i in 0..10_000 {
        queue.push(i).unwrap();           // parks while full
    }
    queue.close();
    assert_eq!(consumer.join().unwrap(), (0..10_000).sum());
});
```

## API

| | blocking | non-blocking | timed |
|---|---|---|---|
| push | `push(T) -> Result<(), PushError<T>>` | `try_push(T) -> Result<(), TryPushError<T>>` | `push_timeout(T, Duration)` |
| pop | `pop() -> Result<T, PopError>` | `try_pop() -> Result<T, TryPopError>` | `pop_timeout(Duration)` |

Plus `close`, `is_closed`, `len`, `is_empty`, `is_full` and `capacity`. Every
failed push hands the item back. `try_*` never fails spuriously: `Full` and
`Empty` are reported only after confirming the queue really was full or empty.

| | `LockFreeQueue` | `BlockingQueue` |
|---|---|---|
| Fast path | one CAS + one `Release` store | one mutex acquisition |
| Waiting | spin, yield, then park | park |
| Capacity | next power of two, minimum 2 | exact |
| Scales with threads | yes | serialises on the mutex |

## Results

Measured on an Apple M4 in million items per second. Higher is better.

| workload | `LockFreeQueue` | crossbeam `ArrayQueue` | `BlockingQueue` |
|---|---|---|---|
| 1 producer + 1 consumer | 90.9 | 83.5 | 14.8 |
| 4 + 4 | 46.2 | 49.7 | 7.1 |
| 8 + 8 (oversubscribed) | 45.8 | 51.8 | 6.9 |

Under contention the lock-free queue moves 6–7× as many items as the mutex
queue and stays within about 10% of crossbeam. crossbeam still wins the
asymmetric shapes.

![Throughput scaling](assets/mpmc_scaling.svg)

The design's point is the idle case. crossbeam's queue has no blocking API, so a
waiting consumer has to spin. It wakes in half a microsecond but burns a whole
core. `LockFreeQueue` parks: it wakes in about 10 µs and uses under 2% of a core.

![Wake latency against idle CPU](assets/wake_latency.svg)

Full tables, methodology and caveats are in [docs/BENCHMARKS.md](docs/BENCHMARKS.md).

## How it works

Each slot carries a sequence number that says which position it is ready
for. A producer claims position `p` with a CAS on `tail` once the slot's
sequence equals `p`, writes, and publishes `p + 1` with a `Release` store. A
consumer claims `p` once it sees `p + 1`, reads, and recycles the slot for the
next lap by storing `p + capacity`. Producers and consumers on different slots
never touch the same cache line.

Waiting threads register in a `WaitQueue` and re-check the queue with a
read-modify-write before blocking. That RMW pairs with the notifier's CAS
through a release sequence, which rules out lost wakeups while keeping the
fast path to one `Relaxed` load when nobody is parked. `close()` sets the top
bit of `tail`, which orders it against every push without an extra flag.

[docs/DESIGN.md](docs/DESIGN.md) has the full argument: the happens-before
diagram, the lost-wakeup proof, why close is a mark bit, and why the minimum
capacity is 2.

## Verification

| | what it establishes |
|---|---|
| [loom](https://docs.rs/loom) | every interleaving of 2–3 threads is free of lost wakeups, close races, spurious `try_*` failures and data races |
| Miri | no undefined behaviour in the `unsafe` slot code: no uninitialised reads, double drops, leaks or aliasing violations |
| proptest | random operation sequences match a `VecDeque` model for capacities 1–17 |
| concurrent tests | exactly-once delivery and per-producer FIFO across many shapes and capacities |
| drop accounting | every item is dropped exactly once, including after wraparound, close and rejection |
| `getrusage` | a parked consumer uses about 50 µs of CPU over 300 ms |

The tests have teeth. Replacing the waiter's RMW with a plain load makes loom
report a deadlock, and removing the minimum capacity makes proptest shrink to a
capacity-1 counterexample.

```sh
cargo test                                               # 82 tests + doctests
cargo test --release --test cpu_burn -- --ignored        # idle CPU check
RUSTFLAGS="--cfg loom" cargo test --release --test loom  # model checking
cargo +nightly miri test --lib --test drop_semantics --test regressions
cargo bench && cargo run --release --example plot        # regenerate charts
```

## Project history

This crate began as a take-home assignment. The submitted version passed its
own tests but had real defects: `try_push` failed spuriously under contention,
non-power-of-two capacities lost items, blocked threads spun forever, and the
benchmarks mostly timed `thread::spawn`. Version 0.2 is the result of auditing
that submission. [DESIGN.md §9](docs/DESIGN.md#9-what-the-original-submission-got-wrong)
lists each defect with the evidence and the fix, and the git history shows the
steps.

## Layout

```text
src/
  queue/lockfree.rs    LockFreeQueue: sequence protocol, parking, close, Drop
  queue/blocking.rs    BlockingQueue
  queue/slot.rs        MaybeUninit slot and its safety contract
  sync/wait_queue.rs   parking and the lost-wakeup argument
  sync/primitives.rs   std/loom shim: the only place sync primitives come from
  sync/pos.rs          63-bit position arithmetic and the closed bit
  sync/backoff.rs      spin / snooze / park backoff
tests/                 loom, proptest, drop accounting, regressions, CPU check
benches/               throughput (criterion) and wake latency (custom harness)
examples/plot.rs       regenerates assets/*.svg from benchmark output
```

## Not yet

* `no_std + alloc` support for `LockFreeQueue`.
* Async `push`/`pop`.
* FIFO fairness among parked threads, which `Condvar` does not guarantee.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
