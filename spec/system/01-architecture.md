# 01 — Architecture

Corvus is **one self-contained Rust binary** with an embedded database, plus an optional self-contained desktop
app that embeds the same core. Nothing else has to be installed or running: no containers, no database server,
no daemon in the agent path.

## Stack

| Concern | Choice | Decision |
|---|---|---|
| Language | Rust stable, edition 2024 | owner direction |
| Storage | SQLite, compiled into the binary, WAL mode, one file | [D0013](../decisions/0013-storage-sqlite.md) |
| Database access / migrations | `rusqlite` (bundled), synchronous services, in-house migrator | [D0014](../decisions/0014-rust-stack-rusqlite.md) |
| Async runtime / HTTP | `tokio`, `axum` (+ `tower`, `tower-http`) | [D0014](../decisions/0014-rust-stack-rusqlite.md) |
| MCP | `rmcp` (official Rust MCP SDK), stdio transport | [D0014](../decisions/0014-rust-stack-rusqlite.md) |
| CLI | `clap` (derive) | [D0014](../decisions/0014-rust-stack-rusqlite.md) |
| Outbound HTTP (Linear, Jira) | `reqwest` with `rustls`, redirects disabled | [D0014](../decisions/0014-rust-stack-rusqlite.md) |
| Logging | `tracing` + JSON formatter to stderr | [D0014](../decisions/0014-rust-stack-rusqlite.md) |
| Dashboard | Svelte 5 (runes) + TypeScript + Vite, inside the desktop app only | [D0020](../decisions/0020-desktop-only-dashboard.md) |
| Desktop | Tauri 2, self-contained (core in-process, UI over IPC, no listener) | [D0020](../decisions/0020-desktop-only-dashboard.md) |

Exact crate versions are pinned by `Cargo.lock` at implementation time; this spec does not pin them.

## Artifacts

| Artifact | What it is | Needs at run time |
|---|---|---|
| **`corvus`** | The binary: CLI, MCP server, inbox hook, wake supervisor, HTTP server, stack commands | nothing |
| **Corvus desktop app** | Tauri 2 bundle: dashboard + core in one process; ships a copy of `corvus` | the OS webview |

Either works without the other. They share one database file and coordinate through it.

## Workspace layout

```
Cargo.toml                  workspace
crates/
  corvus-domain/            PURE: types, validation, presence, name generation, guidance text,
                            source filters → JQL / GraphQL variables, rich-text flattening. No IO.
  corvus-core/              config, clock, db (open, pragmas, migrator), store (SQL), services
                            (Huginn, Muninn, Hermod), background (housekeeping, work-source engine,
                            leader lock), export. Owns all transactions.
    migrations/             NNNN_name.sql — ordered, one-way, embedded
  corvus-jira/              Jira Cloud REST adapter  -- both implement the `WorkSource` trait
  corvus-linear/            Linear GraphQL adapter   -- defined in corvus-core
  corvus-http/              axum: inner data router + outer security layers (no UI assets)
  corvus-mcp/               rmcp server: tool definitions → service calls
  corvus-cli/               bin `corvus`: every subcommand
  corvus-desktop/           Tauri 2 app
ui/                         Svelte 5 dashboard source; built into corvus-desktop only
Makefile                    dev conveniences: test / lint / ui / run
.env.example
```

## Layering (enforced)

```
transports   corvus-http   corvus-mcp   corvus-cli   corvus-desktop(IPC)     parse → call service → serialise
                   │            │            │               │
services     corvus-core::services                         own transactions, audit, hints
                   │
store        corvus-core::store                            SQL only; functions take a connection/transaction
                   │
domain       corvus-domain                                 pure types + rules
```

Rules:

- A transport handler contains no branching on domain state. If it needs an `if` about tasks or messages, the
  logic belongs in a service.
- Store functions never begin or commit transactions; services do.
- `corvus-domain` has no IO dependencies (`rusqlite`, `tokio`, `reqwest`, `std::fs`). CI checks its dependency list.
- Only `corvus-core::db` opens database connections. No other crate depends on `rusqlite`.
- One error type crosses the service boundary: `CorvusError { NotFound, Conflict, Validation, SessionNotActive,
  Unauthorized, Busy, Internal }`. Each transport maps it once ([04](04-interfaces.md#error-model)).

## Processes

Every Corvus process opens the database file itself ([D0015](../decisions/0015-process-topology-single-binary.md)).

| Process | Lifetime | Database | Extra duties |
|---|---|---|---|
| `corvus mcp` | one per agent client, over stdio | read/write | — |
| `corvus <command>` | one-shot | read/write | — |
| `corvus hook` | one-shot, milliseconds | **read-only** | — |
| `corvus watch <agent>` | long-running, one per agent | read/write | — |
| `corvus serve` | long-running, headless; **will not start without a token** | read/write | the way to run Corvus's background work without the desktop app; HTTP API (programmatic); background-leader candidate |
| Corvus desktop app | interactive, stays in the tray | read/write | the dashboard, over IPC; background-leader candidate; never opens a socket |
| Scripts and other tools | any | — (HTTP API, via `corvus serve`) | — |

Consequences worth knowing:

- **Agents need no running service.** Claiming, messaging and waiting work with nothing but the binary and the
  file. `claim_task` reaps expired leases itself, so the queue is correct even if no background loop ever runs.
- **Background loops** (housekeeping, tracker polling) run in exactly one long-lived process at a time
  ([10](10-config-ops.md#background-leader)). With neither `serve` nor the app running, sessions are not
  auto-ended and trackers are not polled until one starts.
- **The HTTP API is opt-in and programmatic.** Only `corvus serve` hosts it, and only with a token. There is
  no browser UI ([D0020](../decisions/0020-desktop-only-dashboard.md)).

## Database access

All in `corvus-core::db`. Decisions: [D0013](../decisions/0013-storage-sqlite.md), [D0017](../decisions/0017-single-writer-concurrency.md).

- **Location**: `CORVUS_DATABASE_URL` (a path or `sqlite:` URL) or, by default, `<state dir>/corvus.db`.
  Directory `0700`, files `0600`. Local disk only.
- **One opener.** `db::open()` sets, on every connection: `journal_mode = WAL`, `foreign_keys = ON`,
  `busy_timeout = 5000`, `synchronous = FULL`, `trusted_schema = OFF`, and verifies the result of each pragma.
- **Connections per process**: one write connection behind a mutex; a small set of read connections opened
  `query_only`. The hook opens a single read-only connection.
- **Write transactions** always start with `BEGIN IMMEDIATE` and never contain network IO, file IO or waiting.
  Read methods never write. A transaction is never upgraded from read to write.
- **Busy handling**: SQLite queues writers for up to `busy_timeout`; on timeout the service retries twice, then
  returns `CorvusError::Busy` (HTTP 503, retryable).
- **Sync core, async edges.** Services are synchronous functions. `corvus_core::Corvus` is the async facade the
  tokio transports use; it runs each call on the blocking pool. CLI commands and the hook call services directly.
- **Version skew**: every write transaction first compares `PRAGMA user_version` with the binary's schema
  version; a mismatch returns a clear "Corvus was upgraded — restart this process" error rather than running
  against a schema it does not know ([02](02-data-model.md#migrations)).

## Time

All timestamps and lease/presence comparisons use a `now` supplied by an injectable `Clock` in `corvus-core`
(system clock in production, manual clock in tests) and bound as a query parameter — never SQL
`CURRENT_TIMESTAMP` / `unixepoch()` in decision logic ([D0007](../decisions/0007-time-source.md)). Stored as
integer UTC milliseconds; serialised as RFC 3339 UTC.

## Identifiers

- UUIDv7 (generated in Rust, stored as text) for sessions, tasks, claims, threads, decisions, artifacts.
- `INTEGER PRIMARY KEY AUTOINCREMENT` for messages, digests, events, hints and source links (monotonic, never
  reused). Nothing treats a message id as a cursor (brief §3 Receipt).
- Agents are keyed by their validated name.

## Platforms

**macOS and Linux only** (owner direction). Windows is out of scope: do not add Windows code paths, build
targets, tests or documentation. Unix facilities (file locks, process groups, signals, permissions, `~/.local/bin`)
may be used directly.

macOS release artifacts are code-signed with the owner's personal signing identity
([plan 0011](../plans/0011-distribution.md)).

## Known gaps (kept current; feeds the final architecture note)

- **Writers are serialised.** SQLite has one write lock, so concurrent claimers queue for the length of one
  short transaction each. They never get the same task and never wait on another agent's lease, but the brief's
  "never block each other" holds only in that sense ([D0017](../decisions/0017-single-writer-concurrency.md)).
- **`CORVUS_DATABASE_URL` is optional**, where brief §6 says required ([D0013](../decisions/0013-storage-sqlite.md)).
- Single machine, single operator: any process running as the same OS user can open the database file. The
  bearer token protects the HTTP surface only. Agent names are claims, not identities.
- Wake-up latency across processes is up to ~250 ms ([D0016](../decisions/0016-wakeups-hints-table.md)).
- Sessions auto-end and trackers sync only while `corvus serve` or the desktop app is running.
- The hook needs `CORVUS_AGENT_NAME` in its environment to know whose inbox to read ([06](06-delivery-helpers.md)).
- Ticket removal or cancellation in Jira or Linear does not cancel tasks (brief §4.9.5).
- Work sources are read-only: nothing is written back to Linear or Jira, so tracker status does not follow task status.
- Duplicate delivery by the wake supervisor after a crash is possible and accepted (brief §4.7).
- There is no dashboard without the desktop app; a headless machine has the CLI and the HTTP API.
- The desktop app's window behaviour is verified by unit tests on its guards plus a manual checklist.
- The database must live on a local filesystem; network filesystems are unsupported.
- The desktop app's single-instance guard listens on a Unix-domain socket in `/tmp` (no network port); see
  [plan 0006](../plans/0006-dashboard-and-desktop.md#unverified--deferred).
