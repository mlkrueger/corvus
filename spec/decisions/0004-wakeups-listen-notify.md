# D0004 — Wake-ups via trigger-emitted `LISTEN/NOTIFY` with periodic re-check

- **Status:** Superseded by [D0016](0016-wakeups-hints-table.md)
- **Date:** 2026-11-13
- **Plans:** [0003](../plans/0003-muninn-hermod.md), [0007](../plans/0007-delivery-helpers.md)
- **Spec:** [system/03](../system/03-services.md#hermod-wake-ups) · [system/02](../system/02-data-model.md#triggers) · P5, brief §4.3

## Problem

`wait_for_inbox` must block cheaply until a message arrives, across processes: the message may be posted by the
server, a CLI command or another agent's MCP process ([D0003](0003-process-topology.md)). Notifications are
hints only (P5): consumers must tolerate lost, duplicated and coalesced hints and must subscribe before reading.

## Options

1. **Polling only** — re-run the unread query every second or so.
2. **In-process broadcast only** — a tokio channel inside one process.
3. **Postgres `LISTEN/NOTIFY`**, emitted by application code after each write.
4. **Postgres `LISTEN/NOTIFY`, emitted by triggers**, plus a periodic re-check in every waiter.

## Trade-offs

### Polling only
- **For:** simplest; cannot lose anything.
- **Against:** latency equals the poll interval; constant query load from every waiting agent; "block until a
  wake-up" becomes fiction.

### In-process broadcast only
- **For:** instant, no database involvement.
- **Against:** does not cross processes, so it only works if every writer and waiter is in the server — ruled
  out by [D0003](0003-process-topology.md).

### NOTIFY from application code
- **For:** cross-process, transactional (delivered on commit), built in.
- **Against:** a code path that forgets to notify silently breaks wake-ups; the brief says a hint is emitted
  when *any* message is added.

### NOTIFY from triggers + re-check
- **For:** no write path can skip the hint; still transactional; the periodic re-check (10 s) and a
  wake-everyone-on-reconnect rule bound the damage of a dropped listener connection, which is the one way
  `NOTIFY` can lose messages.
- **Against:** logic in the database (two small triggers); one extra connection per process for listening;
  waiters wake for hints that turn out not to concern them (cheap: one indexed query).

## Decision

**Option 4.** Triggers on `messages` and `tasks` call `pg_notify('corvus_hint', …)`; each process runs one
listener that fans out to in-process subscribers; every waiter follows subscribe → read → wait → re-read and
re-checks at least every 10 s. Correctness never depends on a hint arriving — hints only make it fast.

## Consequences

- Payloads carry only a routing key (`inbox:<agent>`, `inbox:*`, `tasks`), never state.
- The listener must use a dedicated, non-pooled connection.
- A lost hint costs at most the re-check interval of latency.
- The dashboard does not use hints; it polls (brief §5).

## Discussion

- 2026-11-13 — Initial record.
- 2026-11-13 — SQLite has no LISTEN/NOTIFY. Superseded by D0016 (hints table + data_version).
