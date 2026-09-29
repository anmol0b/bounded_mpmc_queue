# The work-stealing deque

`Worker<T>` / `Stealer<T>` implement the Chase-Lev deque (SPAA 2005) with the
memory orderings Lê, Pop, Cohen and Zappa Nardelli proved correct for C11
("Correct and Efficient Work-Stealing for Weak Memory Models", PPoPP 2013),
adapted to the C++20 rules Rust follows. This is the structure behind
work-stealing schedulers such as Rayon and Tokio.

## 1. Shape

```text
            steal (thieves, FIFO)                push / pop (owner, LIFO)
                    │                                     │
                    ▼                                     ▼
   ... │ top │ top+1 │ ... │ bottom−1 │ bottom │ ...        buffer (power of two)
```

* `top`: next index to steal. Only ever increases, by a successful CAS.
* `bottom`: one past the owner's newest element. Written only by the owner.
* `buffer`: a ring of slots. Written only by the owner, when it doubles.

Indices are `usize` and wrap; every comparison is `bottom.wrapping_sub(top)`
as a signed number. A unit test and the proptest model start indices a few
positions below `usize::MAX`.

## 2. The algorithm and its orderings

```text
push(x):  b = bottom (Relaxed); t = top (Acquire)
          if full: grow
          slot[b] = x (Relaxed); bottom = b + 1 (Release)

pop():    b = bottom − 1; bottom = b (Release)
          fence(SeqCst)                                ← the pop fence
          t = top (Relaxed)
          if b − t < 0: bottom = b + 1 (Release); empty
          x = slot[b]
          if b − t > 0: return x                       (no thief can reach b)
          last element: CAS top t → t + 1 (SeqCst); bottom = b + 1 (Release)
                        return x if the CAS won

steal():  t = top (Acquire)
          fence(SeqCst)                                ← the steal fence
          b = bottom (Acquire)
          if b − t ≤ 0: empty
          buffer (Acquire); x = slot[t] (Relaxed)      may be stale; never used unless…
          CAS top t → t + 1 (SeqCst)                   …this wins; else Retry
```

| Point | Ordering | Why |
|---|---|---|
| push: load `top` | Acquire | Pairs with a thief's CAS on `top`, so the thief's read of slot `t` happens before the owner overwrites that slot on the next lap |
| push: store `bottom` | Release | Publishes the slot and the element behind it |
| pop: store `bottom` | Release | See §4: every `bottom` store is Release |
| **pop fence** | `fence(SeqCst)` | Store-to-load ordering: the reservation must be visible before `top` is read. Acquire/release cannot forbid store buffering |
| pop: CAS on the last element | SeqCst | Arbitrates the last element against thieves |
| steal: load `top` | Acquire | Pairs with other CASes on `top` |
| **steal fence** | `fence(SeqCst)` | Orders "read `top`" before "read `bottom`" and places the thief in the SeqCst fence order relative to the pop fence |
| steal: load `bottom`, `buffer` | Acquire | Makes the slot, the element, and a grown buffer's copies visible |
| steal: CAS on `top` | SeqCst | The only way to claim index `t` |
| grow: publish the new buffer | Release | Copies are visible to anyone who loads the new pointer |

### The fence lemma

C++20 [atomics.order]/4.4: if A is coherence-ordered before B on some atomic,
a SeqCst fence X is sequenced before A, and B is sequenced before a SeqCst fence
Y, then X precedes Y in the single total order of SeqCst operations.

Take a pop whose `top` read sees an older value than a steal's `top` read. The
pop fence is sequenced before the pop's read and the steal's read is sequenced
before the steal fence, so the pop fence precedes the steal fence. If the
steal's `bottom` read then saw a value older than the pop's reservation, the
same rule with the roles reversed would put the steal fence before the pop
fence, a contradiction. So the thief sees the reservation, and cannot also
take index `b`. Remove either fence and neither conclusion follows.

## 3. What goes wrong without the fences

The 2005 pseudocode assumes sequential consistency. Implemented literally with
plain loads and stores, on x86 (which allows a store to be delayed past a later
load), starting from `top = 0`, `bottom = 2`:

1. The owner's pop stores `bottom = 1`; the store sits in its store buffer.
2. The owner loads `top = 0` and sees two elements, so it will take index 1
   without a CAS.
3. A thief steals index 0: `top` becomes 1.
4. A thief loads `top = 1` and `bottom = 2` (the owner's store is still
   buffered), CASes `top` to 2 and takes index 1.
5. Both the owner and the thief own element 1.

ARM and Power allow more reorderings, each closed by one of the orderings in
the table: a thief reading `bottom` before `top` (closed by the steal fence), a
slot store becoming visible after `bottom` (Release on `bottom`), a thief
reading a grown buffer's slots before the copies (Release on the buffer), and
the owner overwriting a slot before a thief finished reading it (Acquire on
`top` in push).

**Loom reproduces the x86 trace.** Built with either fence removed
(`--cfg parkring_mutant="deque_no_pop_fence"` or `"deque_no_steal_fence"`),
the model in which the owner pops while a thief steals twice fails with

```text
assertion `left == right` failed: an element was taken twice: [0, 1, 1]
```

and the three-party model fails the same way. With both fences every model
passes. CI builds each mutant and requires that failure.

## 4. Why every `bottom` store is Release

Lê et al. publish with a standalone `fence(Release)` in push and plain stores
elsewhere. In C11 as they used it, later stores by the same thread extended a
release sequence; C++20 (P0982) removed that. Their algorithm survives only
because a release fence covers every later store by the thread, including
pop's. This implementation uses a Release store instead of the fence
(cheaper on AArch64: `stlr` rather than `dmb; str`), so every store to `bottom`,
including pop's reservation and restore, must be Release too. That gives a
local invariant: a thief that acquire-reads any `bottom` value sees every slot
written before it.

## 5. Elements are pointers, and why

A thief reads slot `t`, then CASes `top`. Between the two, another thief can
win index `t` and the owner can push index `t + capacity` into the same slot:
the thief's read races with the owner's write. The thief's CAS then fails and
it discards what it read, but the race still happened.

crossbeam-deque performs that read non-atomically and documents it as
"technically speaking a data race and therefore UB". Here every slot is an
`AtomicPtr`, so the race is on an atomic and is harmless: a stale or null
pointer is only a value, dereferenced only after the CAS makes the thread its
owner. The cost is that an element must be one pointer: `Worker<T>` boxes each
value (the pool pushes pointers to jobs it has already allocated, so it pays
nothing). Loom and Miri both check this path, which they would reject with a
non-atomic read.

## 6. Growing, and a bug Miri found

The deque only grows, by doubling. A thief may still hold a pointer to the old
buffer, so old buffers are retired and freed only when the deque is dropped.
Their total size is less than the current buffer's, so memory stays within 2×
the peak.

The first version stored retired buffers as `Box<Buffer>`, converting the raw
pointer back with `Box::from_raw` when retiring it. Miri rejected it:

```text
Undefined Behavior: Data race detected between (1) non-atomic read on thread
`unnamed-6` and (2) retag write of type `parkring::deque::Buffer` on thread
`exactly_once_ma`
```

Creating a `Box` asserts unique ownership, a retag that the aliasing model
treats as a write, while a thief was still reading the buffer's header through
its raw pointer. The retire list now holds raw pointers, and ownership is
reclaimed only in `Drop`, when no thief can remain. Loom could not have found
this: it checks memory orderings, not Rust's aliasing rules. The two tools are
complementary, and each caught a real bug in this repository.

## 7. Verification

| | |
|---|---|
| loom (`src/deque/loom_tests.rs`, raw tokens so a double take is an assertion, not a double free) | pop vs two steals, the last element, growth during a steal, slot reuse across laps with wrapped indices, two thieves plus the owner, a stealer outliving the worker; both fence mutants must fail |
| Miri | unit tests, the proptest model, exactly-once under concurrency, drop accounting |
| proptest | single-threaded behaviour matches a `VecDeque` (push/pop at the back, steal from the front, no retries), capacities 1–8, indices near `usize::MAX` |
| exactly-once | 1, 2, 4 and 8 thieves against an owner that pushes and pops; every item taken once, and each thief's items strictly increasing |
| drop accounting | leftovers after several growths, a stealer keeping the deque alive, a concurrent run |

**What loom cannot tell us here.** Loom models `fence(SeqCst)` by joining a
global clock, which is stronger than C++20 (where SeqCst fences order the
total order but create no happens-before edges), so it could accept code that
is correct only under the stronger model. The fence lemma above uses only
[atomics.order]/4.4. Loom also treats SeqCst accesses as AcqRel, so the CASes'
SeqCst ordering is not what it verifies; the proof does not rely on it either.
