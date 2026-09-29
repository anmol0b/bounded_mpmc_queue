# SCQ: the fetch-add queue

`ScqQueue` implements Nikolaev's SCQ ("A Scalable, Portable, and
Memory-Efficient Lock-Free FIFO Queue", DISC 2019), cross-checked against the
author's reference code (`lfring_cas1.h` and the `scqd` benchmark queue in
`rusnikola/lfqueue`). This document explains the structure, the orderings,
where this implementation deliberately departs from the paper, and one flaw
the tests found in the paper's threshold bound.

## 1. Why another queue

`LockFreeQueue` claims a slot with a CAS on `head` or `tail`. Under contention
most of those CASes fail and retry, and a producer preempted between its CAS
and its publish stalls the consumer of that slot until it runs again, so the
queue is not formally lock-free (DESIGN.md §7).

SCQ claims positions with `fetch_add`, which never fails. A CAS remains only on
the one ring entry a position maps to, where it is rarely contended. And no
operation ever waits on a particular other thread: a consumer that reaches a
position whose producer has not published yet waits briefly, then invalidates
the position, and the producer simply takes another one. That makes SCQ
lock-free.

## 2. Structure

```text
ScqQueue<T>
├── data: [UnsafeCell<MaybeUninit<T>>; n]    the items
├── fq: IndexRing (2n entries)                indices of free cells
└── aq: IndexRing (2n entries)                indices of full cells, FIFO

push:  i = fq.dequeue()  →  data[i] = item  →  aq.enqueue(i)
pop:   i = aq.dequeue()  →  item = data[i]  →  fq.enqueue(i)
```

An index is always in exactly one place: the free ring, the allocated ring, or
the thread that dequeued it. That ownership is what makes the unsynchronised
data cells safe. Each ring holds at most `n` indices in `2n` entries, so a ring
enqueue can never find the ring full.

### Entries

Each ring entry is one word: `cycle | IsSafe | index`, where `index` is a data
index or ⊥ (all ones). Position `p` lives in entry `slot(p)` and belongs to lap
`cycle(p) = p / 2n`. Positions are monotonic 63-bit counters (the top bit of
the allocated ring's `tail` is the closed flag), so cycles only grow, plain
integer comparison is correct, and no entry value ever recurs (no ABA). That is
why `ScqQueue` exists only on 64-bit targets: a 31-bit position space would
wrap within seconds.

`slot` rotates the low bits of the position (the paper's `cache_remap`), so
threads that took consecutive positions from the same `fetch_add` touch
different cache lines.

### Ring enqueue

```text
loop:
  t = tail.fetch_add(1)                    closed flag set → fail
  e = entry[slot(t)]
  if cycle(e) < cycle(t) and index(e) = ⊥ and (IsSafe(e) or head ≤ t):
      CAS e → (cycle(t), safe, index)      publish; on failure re-examine
      threshold.swap(3n − 1)
      return
  // otherwise the entry is occupied or a later lap passed it: take a new t
```

### Ring dequeue

```text
if threshold < 0 and tail ≤ head: return empty            (see §4)
loop:
  h = head.fetch_add(1)
  e = entry[slot(h)]
  cycle(e) = cycle(h)      → take index(e), set index to ⊥ (fetch_or)
  cycle(e) > cycle(h)      → a later lap passed; nothing here
  older, index ⊥           → wait briefly for an in-flight enqueuer, then
                              CAS the entry to (cycle(h), ⊥) so it cannot land
  older, holds an index    → a slow consumer has not taken it: mark unsafe
  if tail ≤ h + 1: move tail up to h + 1 (catchup); return empty
  if threshold.fetch_sub(1) ≤ 0: return empty
```

`IsSafe` handles a slow consumer: a dequeuer that finds last lap's item still
there marks the entry unsafe, and an enqueuer may only reuse an unsafe entry if
`head ≤ t`, that is, if the dequeuer for its position has not passed yet.

## 3. Orderings

No operation uses `SeqCst`. Every argument goes through acquire/release and the
fact that a read-modify-write always reads the latest value, which is what loom
models (loom treats `SeqCst` accesses as `AcqRel`).

| Operation | Ordering | Why |
|---|---|---|
| `tail.fetch_add`, `head.fetch_add` | Relaxed | No data flows through them; an RMW still reads the latest value, including the closed flag |
| entry load | Acquire | Synchronises with the enqueuer's publish (data visible), or with a dequeuer's unsafe mark (its `head` increment visible) |
| entry publish CAS | AcqRel | Release publishes the data write (allocated ring) or the finished data read (free ring) |
| entry advance / mark CAS | AcqRel | Release makes the dequeuer's `head` increment visible to enqueuers that later see the entry |
| consume `fetch_or` | Relaxed | The data was acquired by the load; as an RMW it continues every release sequence on the entry |
| `head` load in the IsSafe check | Relaxed | Ordered by the Acquire entry load above |
| threshold swap | AcqRel | The word parked consumers re-check (§5) |

The data handoff chain is: producer writes the cell → publish on `aq` (Release)
→ consumer's entry load (Acquire) → consumer reads the cell → publish on `fq`
(Release) → next producer's entry load (Acquire) → next write. This is the
Vyukov publish/recycle pair, split across two rings.

## 4. Where this implementation departs from the paper

**Threshold as a hint, not proof.** The paper's `Threshold` starts at `3n − 1`,
is reset by every successful enqueue and decremented by every failed dequeue
attempt; when negative, dequeuers return empty without claiming a position.
It exists to stop dequeuers from racing ahead of a slow enqueuer forever.

Its `3n − 1` bound counts *positions* between `head` and a new item. It does
not count dequeuers that claimed a position *before* the reset and decrement
*after* it, and their number is bounded by the thread count, not by `n`. The
reference benchmark uses rings of 65,536 entries, where a handful of threads
can never exhaust the threshold. With capacity 1 (threshold 2) and three
producers plus three consumers, they can: the stress test hung in about one run
in four with an item sitting at exactly `head` and the threshold at −1, so
every dequeuer exited immediately and every parked thread slept forever.

The fix: a negative threshold is trusted only if `tail ≤ head` as well, and the
wake condition for parked threads is `tail > head`, ignoring the threshold's
value. A negative threshold still limits every dequeue call to one iteration,
so the livelock protection remains. The capacity-1 test now passes 60 of 60
runs (it failed 11 of 40), and a regression test runs the scenario 200 times
with a watchdog.

**Threshold reset is a swap.** The paper resets with "load; if not `3n − 1`,
store". A stale load can skip the store, and more importantly a plain store is
not an RMW, so parked consumers could not pair with it. Here every successful
enqueue does `threshold.swap(3n − 1, AcqRel)`, and a parked consumer's wake
check is `threshold.fetch_add(0, AcqRel)`. The two are RMWs on one word, so
the same release-sequence argument as the Vyukov queue's parking (DESIGN.md §4)
rules out a lost wakeup. The ring's `tail` cannot serve as that word: its
`fetch_add` happens *before* the item is published.

**Always CAS when skipping.** When a dequeuer finds an entry that already
looks the way it wants to leave it (last lap's item, already marked unsafe),
the reference code skips the write. But that CAS is also what publishes the
dequeuer's `head` increment: an enqueuer that later reads the entry with
Acquire relies on it to see `head > t` and stay out of a position whose
dequeuer has already passed. Without it, a stale entry read, a slow consumer
consuming the old item, and a new enqueuer that reads a stale `head` can
combine to publish an item into a position nobody will dequeue. Here the
dequeuer always performs the CAS; a failed CAS returns the current value to
re-examine. The cost is one extra RMW in a rare path.

This one is argued, not machine-checked. The failing schedule needs four
threads (the dequeuer that marked the entry, the slow consumer, the dequeuer
that skips, and the enqueuer), beyond what loom can explore here. A build with
the shortcut restored passed every loom model, so the models do not
distinguish the two; the change is kept on the strength of the C11 argument.

**Bounded wait before invalidating.** Like the reference code, a dequeuer that
finds an empty entry while an enqueuer has already claimed its position waits a
bounded number of re-reads before invalidating it. This keeps near-empty
queues from churning while preserving lock-freedom.

## 5. Close, parking and `Drop`

`close` sets the top bit of the allocated ring's `tail` with `fetch_or`. A push
only succeeds at a position it claimed before that bit in `tail`'s modification
order; a later claim sees the bit, takes its item back out of the cell (never
published, so still owned) and returns the index to the free ring. After
observing the flag, consumers dequeue in *drain* mode, ignoring the threshold,
until `head` reaches `tail`.

Parking reuses `WaitQueue` (futex or condvar). A parked consumer re-checks with
an RMW on the allocated ring's threshold; a parked producer on the free ring's.

`Drop` has exclusive access, so no operation is in flight: every index is in
one ring, and consumed entries read ⊥. It drops the cells whose indices are
still in the allocated ring.

## 6. What is weaker than `LockFreeQueue`

`try_push` can return `Full` while a pop has removed an item but not yet
returned its cell to the free ring. The Vyukov queue avoids that by spinning
on the popper, which is exactly the wait SCQ refuses to do. Under concurrency
`try_pop` can likewise return `Empty` when its attempt ran out of threshold
while an item was being published. With one thread, `Full` and `Empty` are
exact, so the proptest model applies unchanged.

SCQ does about twice the atomic operations of the Vyukov queue per item and
uses four words of ring per data cell.

## 7. Verification

| | |
|---|---|
| Shared suite | every behaviour test (`queue_tests!`), drop accounting, exactly-once and per-producer FIFO grid, Send/Sync, idle CPU |
| proptest | random operation sequences against `VecDeque`, capacities 1–17 |
| Unit tests | packing round trips, `slot` is a bijection and spreads positions across lines, threshold exhaustion and catchup on both rings, positions near 2^62 |
| Regression | capacity 1 with 3 + 3 threads, 200 rounds, watchdog |
| loom (`tests/loom_scq.rs`) | lost wakeups on both rings, close waking both sides, close/push linearizability, concurrent claims, multi-lap FIFO, an empty pop invalidating a push, threshold exhaustion, two parked consumers, cross-thread `Drop` |

**Mutation check.** Restoring the paper's conditional threshold reset
(`--cfg parkring_mutant="scq_conditional_threshold"`) makes loom report a
deadlock in the producer-side lost-wakeup model: without the swap, a parked
producer has no RMW to pair with. CI runs that build and requires it to fail.

The two-parked-consumers model is finite at preemption bound 2. The Vyukov
queue's equivalent model is only finite at bound 1, because loom can starve a
preempted producer that consumers wait on. That difference is lock-freedom,
observed by the model checker.
