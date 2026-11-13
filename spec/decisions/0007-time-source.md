# D0007 — Application-supplied time through an injectable clock

- **Status:** Accepted
- **Date:** 2026-11-13
- **Plans:** [0001](../plans/0001-foundation.md), [0002](../plans/0002-huginn-work-queue.md), [0005](../plans/0005-housekeeping.md)
- **Spec:** [system/01](../system/01-architecture.md#time) · brief §4.1, §4.2

## Problem

Lease expiry, presence and auto-end are all comparisons against "now". Tests for them must be fast and
deterministic — nobody should wait 900 seconds for a lease to expire, and sleeping makes tests flaky. Where
does "now" come from?

## Options

1. **The database clock** — `now()` / `clock_timestamp()` in SQL.
2. **The application clock, injected** — services take `now` from a `Clock` trait and pass it into SQL as a
   parameter.
3. **Database clock in production, tiny durations in tests** — configure 1-second leases and sleep.

## Trade-offs

### Database clock
- **For:** one authoritative clock even if processes ran on different machines; nothing to pass around.
- **Against:** untestable without sleeping or patching the database; `now()` is frozen at transaction start,
  which surprises people in long transactions.

### Injected application clock
- **For:** tests set the time and assert exactly at boundaries (lease expires at `t`, not `t−1`); presence stays
  a pure function; no sleeps anywhere in the suite.
- **Against:** processes could disagree if their clocks differed; every time-dependent query needs a `:now`
  parameter; a stray SQL `now()` would be a subtle bug.

### Short durations + sleep
- **For:** no abstraction.
- **Against:** slow, flaky under load, and cannot test exact boundaries.

## Decision

**Option 2.** Corvus is by definition a single-machine system, so all processes share one OS clock and the only
real argument for the database clock does not apply. `corvus-core` has a `Clock` trait (`SystemClock`,
`ManualClock`); services read it once per operation and bind it as a parameter.

## Consequences

- `make lint` greps the migrations and query files for `now()`, `clock_timestamp()` and `CURRENT_TIMESTAMP` in
  decision logic and fails on a hit.
- If Corvus is ever made multi-machine, this decision must be superseded.
- Waiting (`wait_for_inbox`, backoff) uses tokio time, which tests control with paused time.

## Discussion

- 2026-11-13 — Initial record.
- 2026-11-13 — Storage moved to SQLite (D0013). The decision stands; the SQL functions to avoid are now `CURRENT_TIMESTAMP`, `unixepoch()` and `datetime('now')`. The single-machine argument is stronger still: all processes share one file.
