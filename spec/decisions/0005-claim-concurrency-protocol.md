# D0005 — Row locks in fixed order + `SKIP LOCKED` + partial unique indexes

- **Status:** Superseded by [D0017](0017-single-writer-concurrency.md)
- **Date:** 2026-11-13
- **Plans:** [0002](../plans/0002-huginn-work-queue.md), [0005](../plans/0005-housekeeping.md)
- **Spec:** [system/03](../system/03-services.md#lock-protocol) · [system/02](../system/02-data-model.md#claims) · P2, brief §4.2

## Problem

Claim, renew, release, complete, cancel, lease expiry and session end all touch the same three kinds of row and
can run concurrently from different processes. Two claimers must never win the same task or block each other;
no combination may deadlock; cancellation must be safe against a claim appearing at the same moment. The brief
prescribes a consistent lock order: session → claims → tasks.

## Options

1. **Pessimistic row locks** in a fixed order, `FOR UPDATE SKIP LOCKED` for queue scans, unique indexes as a
   backstop.
2. **`SERIALIZABLE` isolation** everywhere, retrying on serialization failure.
3. **Optimistic conditional updates** — `UPDATE tasks SET status='claimed' WHERE id=? AND status='open'` in a
   retry loop.
4. **Advisory locks** keyed by task id.

## Trade-offs

### Fixed-order row locks + SKIP LOCKED
- **For:** claimers skip rows another transaction holds, so they neither wait nor collide — the brief's exact
  requirement; a fixed order makes deadlock impossible by construction; behaviour is easy to reason about and to
  test deterministically (hold a lock, observe the other side).
- **Against:** the order must be respected by every operation forever; cancel naturally wants task-then-claim
  and needs a re-check-and-retry step instead; a claimer can momentarily skip the best task.

### SERIALIZABLE + retry
- **For:** no lock-order discipline; the database finds conflicts.
- **Against:** conflicts surface as aborted transactions, so racing claimers retry and effectively do block each
  other; retry storms under contention; harder to test deterministically; still needs constraints as a backstop.

### Optimistic conditional updates
- **For:** no explicit locks for the simple claim.
- **Against:** N claimers all pick the same best task, one wins, N−1 retry — more contention, not less;
  multi-row operations (cancel cascade, session end) still need real locking.

### Advisory locks
- **For:** flexible.
- **Against:** invisible to the schema, easy to leak, and duplicate what row locks already provide.

## Decision

**Option 1.** Existing rows are locked strictly session → claim → task (same-level rows in id order). The claim
scan uses `FOR UPDATE SKIP LOCKED`. New claims are guarded by two partial unique indexes (one active claim per
task, one per session), which hold even if service code is wrong. `cancel_task` locks claims, then tasks, then
re-reads the claims; if one appeared in between it rolls back and retries rather than locking backwards.

## Consequences

- The lock-order table in system/03 is normative; a new operation must be added to it before it is written.
- A generic retry on `40001`/`40P01` exists only as a backstop; a deadlock in tests is a bug in the protocol.
- Race tests must include a deadlock timeout so a violation fails fast.

## Discussion

- 2026-11-13 — Initial record.
- 2026-11-13 — SQLite has one write lock and no row locks; the row-lock protocol does not apply. Superseded by D0017.
