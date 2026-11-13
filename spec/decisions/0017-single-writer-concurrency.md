# D0017 — Every mutation is one short `BEGIN IMMEDIATE` transaction; unique indexes as the backstop

- **Status:** Accepted
- **Date:** 2026-11-13
- **Supersedes:** [D0005](0005-claim-concurrency-protocol.md)
- **Plans:** [0002](../plans/0002-huginn-work-queue.md), [0005](../plans/0005-housekeeping.md)
- **Spec:** [system/03](../system/03-services.md#lock-protocol) · [system/02](../system/02-data-model.md#claims) · P2, brief §4.2

## Problem

Claim, renew, release, complete, cancel, lease expiry and session end must each be atomic and deadlock-free
when issued concurrently from several processes; two claimers must never win the same task (P2). The brief
describes this in row-lock terms: a consistent lock order (session → claims → tasks) and claimers that "never
block each other". SQLite ([D0013](0013-storage-sqlite.md)) has no row locks — it has one write lock for the
whole database. What is the protocol, and what happens to those two brief statements?

## Options

1. **One `BEGIN IMMEDIATE` transaction per mutation.** Take the write lock up front, do everything, commit.
2. **Deferred transactions that upgrade** — start reading, take the write lock at the first write.
3. **Optimistic compare-and-swap** — autocommit statements like `UPDATE … WHERE status = 'open'`, retried.
4. **Application-level lock table or lock files** layered over SQLite to emulate row locks.

## Trade-offs

### One IMMEDIATE transaction per mutation
- **For:** mutations are strictly serial across all processes, so every operation sees a stable world: two
  claimers cannot pick the same task, a claim cannot appear in the middle of a cancel, a heartbeat and an
  auto-end cannot interleave. Deadlock is impossible — there is one lock and it is never held while waiting for
  another. Easy to reason about and to test.
- **Against:** writers queue. A claimer waits for whichever write is in flight — typically well under a few
  milliseconds — so "never block each other" is true only as "never wait on another agent's lease or work".
  Anything slow inside a write transaction stalls every writer, so the rule "no IO, no waiting inside a write
  transaction" becomes critical.

### Deferred transactions that upgrade
- **For:** readers that turn out not to write never take the lock.
- **Against:** two transactions that both read and then try to write deadlock; SQLite resolves it by failing
  one with `SQLITE_BUSY` regardless of the busy timeout. This is the classic SQLite concurrency bug.

### Optimistic compare-and-swap
- **For:** no explicit transactions for the simplest case.
- **Against:** multi-statement operations (claim = select + insert + update; cancel cascade; session end) are
  not atomic without a transaction; losers retry and collide again.

### Emulated row locks
- **For:** would literally reproduce the brief's lock ordering.
- **Against:** builds a second lock manager on top of the database's own, re-introducing the deadlocks SQLite's
  single lock rules out.

## Decision

**Option 1.** Every mutating service method runs entirely inside one `BEGIN IMMEDIATE` transaction on the
process's single write connection; read-only methods use a reader connection and never write. Transactions are
kept short by rule: all validation before, all network and file IO outside. A `busy_timeout` of 5 s covers
queueing; an exhausted timeout is retried twice, then reported as a transient error. The two partial unique
indexes on `claims` (one active claim per task, one per session) remain, so even a buggy service cannot
double-claim.

## Consequences

- **Deviation from the brief, stated plainly:** concurrent claimers are serialised by the database for the
  duration of one short transaction each. They never receive the same task and never wait on one another's
  leases. The brief's lock order survives as the statement order inside each transaction (session, then
  claims, then tasks) for readability; it is not needed for safety.
- The cancel "re-check and retry" loop from D0005 is gone: the race it handled cannot occur.
- A test fails the build if a write transaction performs network or filesystem IO, and the stress tests assert
  a bound on write-transaction duration.
- Race tests run both multi-threaded and **multi-process** (spawning the real binary), because the guarantee
  is cross-process.

## Discussion

- 2026-11-13 — Created with the move to SQLite. Supersedes D0005.
