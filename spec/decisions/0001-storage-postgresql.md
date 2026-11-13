# D0001 — PostgreSQL is the single store

- **Status:** Superseded by [D0013](0013-storage-sqlite.md)
- **Date:** 2026-11-13
- **Plans:** [0000](../plans/0000-roadmap.md), [0001](../plans/0001-foundation.md)
- **Spec:** [system/01](../system/01-architecture.md#stack) · [system/02](../system/02-data-model.md) · P1, P2, P4, P5, P10

## Problem

Corvus needs one durable store (P1) that several **separate processes** use at once: the server, one MCP process
per agent, CLI invocations and wake supervisors. It must give atomic, deadlock-free claiming where concurrent
claimers "never block each other" (P2, brief §4.2), store-level append-only enforcement (P4), cross-process
wake-ups (P5) and locked migrations (P10). The brief also assumes a storage service: `CORVUS_DATABASE_URL` is
required and the stack scripts "start storage".

## Options

1. **PostgreSQL** in a local podman container.
2. **SQLite** (WAL mode), a file on disk.
3. **An embedded Rust key-value store** (redb, sled, fjall).

## Trade-offs

### PostgreSQL
- **For:** row-level locks and `FOR UPDATE SKIP LOCKED` give exactly the claiming semantics the brief
  describes; partial unique indexes, composite foreign keys, triggers and generated columns put invariants in
  the store; `LISTEN/NOTIFY` is a built-in cross-process hint channel; advisory locks cover migrations and
  sync single-flight; many processes connect concurrently with no extra design.
- **Against:** a running service is a dependency (podman, a container, a port, a password); heavier than a
  file for a "local-first" tool; the desktop app cannot be a zero-setup single binary.

### SQLite
- **For:** zero setup, one file, trivially local-first; ideal for a self-contained Tauri app; triggers and
  partial indexes exist.
- **Against:** one writer at a time — claimers serialise behind the write lock, so "never block each other" is
  only approximately true and the brief's session → claim → task lock ordering has nothing to order; no
  `SKIP LOCKED`; no cross-process notification, so wake-ups from the MCP processes need polling or a second
  IPC channel; `SQLITE_BUSY` handling across five kinds of process becomes a design problem of its own.

### Embedded key-value store
- **For:** pure Rust, fast, no service.
- **Against:** typically single-process; relational constraints, queries, migrations and notifications would
  all be hand-built. Most of the brief's guarantees would move from the store into application code — the
  opposite of what P2 and P4 ask for.

## Decision

**We chose PostgreSQL (17 or newer).** The brief's hardest requirements — multi-process access, non-blocking
atomic claims with an explicit lock order, store-enforced invariants, and cross-process wake-ups — are native
Postgres features and would each be a workaround on SQLite. The cost is one container, which the stack scripts
hide behind `make up`.

## Consequences

- podman is a prerequisite; `scripts/up.sh` owns the container lifecycle.
- The desktop shell is a client of a running stack, not a self-contained app ([D0008](0008-tauri-shell-role.md)).
- Tests need a real Postgres; `make test` ensures it is up.
- Revisit if a zero-dependency single-binary distribution becomes a goal; that would mean SQLite plus a
  replacement for `LISTEN/NOTIFY` and a re-think of [D0003](0003-process-topology.md).

## Discussion

- 2026-11-13 — Initial record. This is the most consequential choice in the design; flagged to the owner for review.
- 2026-11-13 — The owner requires distribution as a self-contained Rust binary with no additional containers. PostgreSQL in podman cannot meet that. Superseded by D0013 (SQLite).
