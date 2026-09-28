# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- **Breaking:** the crate is renamed from `bounded_mpmc_queue` to `parkring`.
  Replace `use bounded_mpmc_queue::…` with `use parkring::…`.
- The repository is a Cargo workspace; benchmarks and chart generation live in
  the unpublished `crates/parkring-bench`.
- Parked threads sleep on a futex: `futex(2)` on Linux and Android,
  `__ulock_wait`/`__ulock_wake` on macOS. Other platforms, and builds with
  `--cfg parkring_force_condvar`, keep the `Mutex` + `Condvar` parker. The
  crate now depends on `libc` on those three targets.

### Performance
- Median wake latency of a parked consumer on an Apple M4 drops about 10%
  (8.5–8.8 µs against 9.4–10.0 µs), with a lower p99.

### Added
- Loom models for `close` after one of two parked consumers is woken, and for
  a timed pop racing a push. CI runs the loom suite against both parkers.
- CI tests on Windows and type-checks FreeBSD, both on the portable parker.

## [0.2.0] - 2026-09-28

A production-hardening pass over the original take-home submission. See
[docs/DESIGN.md](docs/DESIGN.md#9-what-the-original-submission-got-wrong) for
the audit that motivated it.

### Added
- Spin-then-park waiting in `LockFreeQueue`: blocked `push`/`pop` spin, yield,
  then park on a condition variable instead of burning a core.
- `close()` on both queues. Pushes fail and return the item; pops drain the
  remaining items, then report `PopError`. All blocked threads are woken.
- `push_timeout` and `pop_timeout` on both queues.
- `len`, `is_empty`, `is_full`, `capacity`, `is_closed`, and `Debug`.
- Error types per operation (`PushError`, `TryPushError`, `PushTimeoutError`,
  `PopError`, `TryPopError`, `PopTimeoutError`). Every push error returns the item.
- `BoundedQueue` covers the full API, is object-safe, and both queues implement it.
- Verification: 12 loom models, Miri over the unsafe code, a proptest model
  against `VecDeque`, exactly-once and per-producer FIFO checks, drop
  accounting, and a CPU-usage test for parked threads.
- Benchmarks against crossbeam's `ArrayQueue` and `std::sync::mpsc::sync_channel`,
  a wake-latency benchmark, and an example that regenerates every chart.
- CI: fmt, clippy, tests on Linux and macOS, MSRV, docs, loom, Miri.

### Changed
- **Breaking:** `push`/`pop` return `Result`; `try_pop` returns `Result`.
- **Breaking:** types are re-exported at the crate root; internal modules are private.
- **Breaking:** `BoundedQueue::new` removed; construct through the concrete type.
- `LockFreeQueue` capacity is rounded up to a power of two, minimum 2.
- Slots store `MaybeUninit<T>`; `Drop` releases exactly the unconsumed items.
- Cache padding is 128 bytes on x86-64 and AArch64.
- `BlockingQueue` notifies after releasing its lock.

### Fixed
- `try_push`/`try_pop` no longer fail spuriously under contention.
- Non-power-of-two capacities no longer lose items or deadlock.
- Capacity 1 no longer overwrites a live item.
- Blocked threads no longer spin indefinitely on an idle queue.

## [0.1.0]

Original take-home submission: `BlockingQueue` and `LockFreeQueue` with
criterion benchmarks.
