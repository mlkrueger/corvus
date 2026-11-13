# D0016 — Wake-ups via a trigger-written hints table, detected by `data_version`

- **Status:** Accepted
- **Date:** 2026-11-13
- **Supersedes:** [D0004](0004-wakeups-listen-notify.md)
- **Plans:** [0003](../plans/0003-muninn-hermod.md), [0007](../plans/0007-delivery-helpers.md)
- **Spec:** [system/03](../system/03-services.md#hermod-wake-ups) · [system/02](../system/02-data-model.md#triggers) · P5, brief §4.3

## Problem

`wait_for_inbox` must block cheaply until something relevant happens, even when the message is written by a
different process ([D0015](0015-process-topology-single-binary.md)). SQLite has no `LISTEN/NOTIFY`. The brief
requires that a hint be emitted when any message is added, carrying the recipient or "everyone", and when any
task changes; that hints are only hints (P5); and that consumers subscribe before reading.

## Options

1. **Re-run the inbox query on a timer** (plain polling).
2. **Poll `PRAGMA data_version`** — a counter SQLite bumps for a connection whenever *another* connection, in
   any process, commits — and re-read state when it changes.
3. **Hints table + `data_version`.** Triggers append a routing key to a small `hints` table in the same
   transaction; waiters detect commits with `data_version` and read only the new hint rows.
4. **File-system watching** of the database's WAL file.
5. **A local socket bus** — each waiter listens on a socket; writers notify all registered sockets.

## Trade-offs

### Plain polling
- **For:** trivial; cannot miss anything.
- **Against:** every waiter runs a real query several times a second forever; latency equals the interval.

### `data_version` only
- **For:** detection costs almost nothing (no query, no disk read in the common case); works across processes
  with no extra moving parts; nothing can forget to notify, because it observes commits themselves.
- **Against:** untargeted — every commit wakes every waiter; the hint does not "carry the recipient".

### Hints table + `data_version`
- **For:** all of the above, plus targeted hints exactly as the brief words them, written by triggers so no
  code path can skip them and committed atomically with the change; waiters ignore traffic that is not theirs;
  hints are testable as data.
- **Against:** one extra tiny insert per message or task change; the table needs pruning; waking latency is
  bounded by the detection interval (250 ms) rather than being instant.

### File-system watching
- **For:** event-driven, no interval.
- **Against:** platform-specific behaviour, coalescing and occasional silence, especially for memory-mapped WAL
  writes; needs the same fallback polling anyway.

### Local socket bus
- **For:** instant and targeted.
- **Against:** a registry of live listeners, stale sockets after crashes, permissions, and a second source of
  truth about who is waiting — a lot of machinery for a hint.

## Decision

**Option 3.** Triggers on `messages` and `tasks` insert `inbox:<agent>`, `inbox:*` or `tasks` into `hints`.
Each process's Hermod component checks `data_version` every 250 ms while it has subscribers, reads hint rows
past its cursor, and wakes matching subscribers. Every waiter still follows subscribe → read → wait → re-read
and re-checks at least every 10 s, so correctness never depends on a hint.

## Consequences

- Worst-case wake latency is about 250 ms; acceptable for agents, and the interval is one constant.
- `hints` is pruned to the most recent 1,000 rows by housekeeping and on open; a waiter whose cursor has been
  pruned past is simply woken (a hint may have been missed).
- `hints` is plumbing, not history: it is exempt from the append-only rule and is never exported.
- The dashboard does not use hints; it polls (brief §5).

## Discussion

- 2026-11-13 — Created with the move to SQLite. Supersedes D0004; keeps its trigger-emitted, hint-only stance.
