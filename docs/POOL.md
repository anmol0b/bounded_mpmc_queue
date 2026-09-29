# The work-stealing pool

`ThreadPool` is deliberately small: its job is to show the crate's pieces
working together as a real scheduler, not to replace Rayon.

```text
 outside threads ──install/spawn──▶ injector (LockFreeQueue<JobRef>)
                                        │
   worker 0      worker 1     ...       ▼
   ┌──────┐      ┌──────┐          every worker: pop own deque
   │deque │◀─────│steal │          → steal from others (random start)
   └──────┘      └──────┘          → take from the injector
       idle workers spin, then park on the futex wait queue
```

## Jobs

A job is a pointer to a `#[repr(C)]` header holding its `execute` function,
so any job type travels through the deques and the injector as one pointer.

* **`spawn`** allocates a `HeapJob` holding the closure. From inside a worker
  it goes onto that worker's deque; from outside, into the injector.
* **`join(a, b)`** puts `b` in a `StackJob` *in the caller's own frame*, pushes
  a pointer to it, runs `a`, then pops: if the pointer comes back, nobody stole
  `b`, and it runs inline; otherwise it helps with other work until `b`'s latch
  is set. No allocation per join. The frame cannot be popped while a thief may
  still run `b`: the caller waits for the latch, and setting the latch is the
  executing thread's last access to the job.
* **`install`** from outside the pool injects a `StackJob` and blocks on a
  `LockLatch`. The latch's mutex and condvar live behind an `Arc` that the
  setter clones *before* setting, because the waiter may return and free its
  frame the moment it sees the flag.

**Latches take raw pointers.** The first version's `Latch::set` took
`&self`. CI's Miri run reported "deallocating while item is strongly
protected": a `&Self` argument stays protected until the function returns, but
the waiting thread may see the flag and pop the frame holding the latch while
`set` is still running. Freeing protected memory is undefined behaviour under
Rust's aliasing model, even if `set` never touches it again. `set` now takes
`*const Self` and `StackJob::execute` never holds a `&Self` across it, the
same approach Rayon takes. A local Miri run across 12 scheduling seeds passes.

Panics: a panic in `a` is held until `b` has finished (`b` may be borrowing the
frame on another thread), then resumed; a panic in `b` is carried back through
the job's result slot. A panic in a spawned job aborts the process, as in
Rayon, because nobody is waiting to receive it.

## Sleeping without losing work

Workers scan (own deque, steals, injector); after a failed scan they spin,
then park on the crate's `WaitQueue`. Before scanning they snapshot an
`events` counter; they park only until it changes, re-checking it with an
`AcqRel` RMW. Injecting a job bumps `events` with an RMW and notifies, which
is the same release-sequence pairing as the queues' parking (DESIGN.md §4): an
injected job is never stranded.

Pushes onto a worker's own deque wake a sleeper only if `has_waiters()` (a
`Relaxed` load) says one may exist. A missed wake here loses parallelism, not
work: the pushing worker is awake and will run its own job.

## Shutdown

`Drop` sets a `TERMINATE` bit in `events` and wakes everyone. A worker exits
only after a scan that found no work anywhere and lost no steal race. Jobs
spawned by running jobs land on that worker's own deque, which it drains
before it can make such a scan. So dropping a pool runs every spawned job to
completion.

## Known limitations

* A worker waiting in `join` for a stolen `b` helps with other work or spins;
  it does not park. Rayon parks here with per-worker sleep states.
* Local pushes wake sleepers best-effort (above).
* No scoped spawn, no thread-count configuration beyond `new(n)`.

## Verification

* Tests: `fib` matches the sequential result at 1, 2 and 8 threads; 10,000
  spawns all run before `drop` returns, including jobs spawned by jobs; a
  5,000-deep chain of joins; panics on either side of a join; concurrent
  `install` from outside threads; `install` from inside runs inline. The suite
  passed 150 repeated runs.
* Loom: a job injected while the only worker may be parking is not lost; a
  spawned job runs exactly once before `drop` returns; a join whose second half
  may be stolen by a second worker.
* Miri: the pool tests, covering the stack-allocated jobs and latches.
* An idle pool of four workers uses about 35 µs of CPU over 300 ms.
