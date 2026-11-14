# Decisions

One record per significant decision. Rules: [`/AGENTS.md` §4](../../AGENTS.md#4-decisions). Template:
[`_template.md`](_template.md). Small defaults live in [`../system/11-defaults.md`](../system/11-defaults.md).

Every record answers, in order: **the problem → the options → the trade-offs of each → what was decided and why.**
Accepted records are immutable apart from status, links and the Discussion log; change a decision by superseding it.

## Current decisions

| ID | Decision | Status | Used by plans |
|---|---|---|---|
| [D0006](0006-idempotency-keys.md) | Per-table `(creator, idempotency_key)` unique index | Accepted | [0002](../plans/0002-huginn-work-queue.md), [0003](../plans/0003-muninn-hermod.md) |
| [D0007](0007-time-source.md) | Application-supplied time through an injectable clock | Accepted | [0001](../plans/0001-foundation.md), [0002](../plans/0002-huginn-work-queue.md), [0005](../plans/0005-housekeeping.md) |
| [D0010](0010-append-only-enforcement.md) | Append-only enforced by triggers | Accepted | [0001](../plans/0001-foundation.md), [0003](../plans/0003-muninn-hermod.md) |
| [D0011](0011-work-source-abstraction.md) | One work-source engine; Linear and Jira are adapters; Linear ships in v1 | Accepted | [0000](../plans/0000-roadmap.md), [0008](../plans/0008-work-sources-and-linear.md), [0009](../plans/0009-jira-source.md) |
| [D0012](0012-comment-tracking.md) | Comment tracking by order key + tie set | Accepted | [0008](../plans/0008-work-sources-and-linear.md), [0009](../plans/0009-jira-source.md) |
| [D0013](0013-storage-sqlite.md) | SQLite, embedded in the binary, is the single store | Accepted | [0000](../plans/0000-roadmap.md), [0001](../plans/0001-foundation.md), [0011](../plans/0011-distribution.md) |
| [D0014](0014-rust-stack-rusqlite.md) | rusqlite with synchronous services; tokio + axum + rmcp + clap at the edges | Accepted | [0001](../plans/0001-foundation.md), [0004](../plans/0004-transports.md) |
| [D0015](0015-process-topology-single-binary.md) | One binary; every local process opens the database; the HTTP server is optional | Accepted | [0000](../plans/0000-roadmap.md), [0001](../plans/0001-foundation.md), [0004](../plans/0004-transports.md), [0007](../plans/0007-delivery-helpers.md), [0011](../plans/0011-distribution.md) |
| [D0016](0016-wakeups-hints-table.md) | Wake-ups via a trigger-written hints table, detected by `data_version` | Accepted | [0003](../plans/0003-muninn-hermod.md), [0007](../plans/0007-delivery-helpers.md) |
| [D0017](0017-single-writer-concurrency.md) | Every mutation is one short `BEGIN IMMEDIATE` transaction; unique indexes as the backstop | Accepted | [0002](../plans/0002-huginn-work-queue.md), [0005](../plans/0005-housekeeping.md) |
| [D0020](0020-desktop-only-dashboard.md) | The dashboard exists only in the desktop app; there is no browser UI | Accepted (spike pending) | [0000](../plans/0000-roadmap.md), [0006](../plans/0006-dashboard-and-desktop.md), [0011](../plans/0011-distribution.md) |
| [D0021](0021-agent-adapters.md) | Agent tools are supported through declarative adapters; Codex is the first built-in | Accepted | [0007](../plans/0007-delivery-helpers.md) |
| [D0022](0022-desktop-single-instance-local-socket.md) | The desktop app's single-instance guard may use a local OS rendezvous; it never opens a network listener | Accepted | [0006](../plans/0006-dashboard-and-desktop.md) |

## Superseded (kept as history)

| ID | Decision | Superseded by | Why |
|---|---|---|---|
| [D0001](0001-storage-postgresql.md) | PostgreSQL is the single store | [D0013](0013-storage-sqlite.md) | Must ship as a self-contained binary, no containers |
| [D0002](0002-rust-service-stack.md) | tokio + axum + sqlx + rmcp + clap | [D0014](0014-rust-stack-rusqlite.md) | Database library followed from PostgreSQL |
| [D0003](0003-process-topology.md) | MCP/CLI/watch use the database directly, hook and UI use HTTP | [D0015](0015-process-topology-single-binary.md) | Embedded database: hook needs no server; scripts become subcommands |
| [D0004](0004-wakeups-listen-notify.md) | Wake-ups via `LISTEN/NOTIFY` | [D0016](0016-wakeups-hints-table.md) | SQLite has no `LISTEN/NOTIFY` |
| [D0005](0005-claim-concurrency-protocol.md) | Row locks in fixed order + `SKIP LOCKED` | [D0017](0017-single-writer-concurrency.md) | SQLite has one write lock, no row locks |
| [D0008](0008-tauri-shell-role.md) | Tauri as a shell around the served dashboard | [D0018](0018-self-contained-desktop.md) → [D0020](0020-desktop-only-dashboard.md) | Owner chose a self-contained desktop app |
| [D0009](0009-dashboard-frontend.md) | Svelte 5 dashboard served by the server | [D0019](0019-dashboard-frontend-two-carriers.md) → [D0020](0020-desktop-only-dashboard.md) | Same framework, new delivery model |
| [D0018](0018-self-contained-desktop.md) | Self-contained Tauri app with an optional HTTP listener | [D0020](0020-desktop-only-dashboard.md) | Desktop-only dashboard; the app never listens |
| [D0019](0019-dashboard-frontend-two-carriers.md) | Svelte bundle with IPC and HTTP carriers | [D0020](0020-desktop-only-dashboard.md) | No browser host; one carrier |

Next free number: **0023**.
