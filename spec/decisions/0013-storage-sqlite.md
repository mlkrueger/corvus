# D0013 — SQLite, embedded in the binary, is the single store

- **Status:** Accepted
- **Date:** 2026-11-13
- **Supersedes:** [D0001](0001-storage-postgresql.md)
- **Plans:** [0000](../plans/0000-roadmap.md), [0001](../plans/0001-foundation.md), [0011](../plans/0011-distribution.md)
- **Spec:** [system/01](../system/01-architecture.md#database-access) · [system/02](../system/02-data-model.md) · P1, P2, P4, P5, P10

## Problem

Owner direction (2026-11-13): **Corvus must be distributable as a Rust binary with no additional containers or
services.** [D0001](0001-storage-postgresql.md) chose PostgreSQL in a podman container, which violates that
outright. The store must now live inside the binary, and still satisfy P1 (one durable, transactional store),
P2 (atomic claiming), P4 (append-only enforced by the store), P5 (wake-ups) and P10 (versioned migrations) —
while being opened by several processes at once (agents' MCP processes, CLI commands, the desktop app).

## Options

1. **SQLite**, compiled into the binary, WAL mode, one file on disk.
2. **Embedded PostgreSQL** — a crate that unpacks and supervises a private Postgres server.
3. **A pure-Rust embedded store** (redb, fjall, sled).
4. **A pure-Rust SQL engine** (Turso/Limbo and similar SQLite rewrites).

## Trade-offs

### SQLite
- **For:** nothing to install or run — the database is a file; transactions, partial unique indexes, composite
  foreign keys, CHECK constraints, generated columns and triggers all exist, so invariants stay in the store;
  multi-process access on one machine is a supported, well-understood mode (WAL); the most tested database
  there is; tests need no setup at all.
- **Against:** one writer at a time for the whole database — writers queue instead of running in parallel, so
  "claimers never block each other" holds only in the weak sense (each waits for sub-millisecond-to-millisecond
  transactions, never for another agent's work) and the brief's row-lock ordering has nothing to order
  ([D0017](0017-single-writer-concurrency.md)); no `LISTEN/NOTIFY`, so cross-process wake-ups need their own
  mechanism ([D0016](0016-wakeups-hints-table.md)); no regex in CHECK constraints; per-connection pragmas
  (foreign keys) must be set by every opener; requires a local filesystem.

### Embedded PostgreSQL
- **For:** keeps every Postgres feature the first design used.
- **Against:** it is still a separate server: a ~100 MB payload unpacked at first run, child processes to
  supervise, a port or socket, crash recovery of someone else's daemon. "No containers" in letter, not in spirit.

### Pure-Rust embedded store
- **For:** no C dependency; fast.
- **Against:** key-value only, mostly single-process; constraints, queries, migrations and multi-process access
  would all be hand-built — moving the brief's store-level guarantees into application code.

### Pure-Rust SQL engine
- **For:** SQLite semantics without C.
- **Against:** young; multi-process behaviour and durability are not proven to the level this system's
  correctness depends on.

## Decision

**SQLite, bundled (compiled from source into the binary), in WAL mode, at `<state dir>/corvus.db` by default.**
It is the only option that is both genuinely embedded and able to carry the brief's guarantees as constraints
and triggers. The cost — a single writer and no built-in notifications — is handled explicitly in D0017 and
D0016 and is honest to state: Corvus coordinates a handful of agents on one machine, where write transactions
are tiny and rare.

## Consequences

- `CORVUS_DATABASE_URL` becomes **optional** (a path or `sqlite:` URL). This departs from brief §6, where it is
  required; zero-configuration start is the point of this decision.
- Brief wording that assumes a storage service ("start storage") reduces to "create the file and migrate".
- Deviation recorded: racing claimers serialise on SQLite's write lock ([D0017](0017-single-writer-concurrency.md)).
- One function opens connections and sets pragmas; nothing else may open the database.
- The database must be on a local disk (no network filesystems) and all processes on the same host.
- Any process running as the same OS user can open the file; the bearer token guards only the HTTP surface.
- Revisit only if Corvus becomes multi-machine.

## Discussion

- 2026-11-13 — Created after the owner required container-free binary distribution. Supersedes D0001.
