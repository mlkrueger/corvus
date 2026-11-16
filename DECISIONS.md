# Decisions

Every significant design choice in Corvus has a record under [spec/decisions/](spec/decisions/README.md) with the
problem, the options considered, their trade-offs and the reasoning. Smaller defaults where the brief is silent
are in [spec/system/11-defaults.md](spec/system/11-defaults.md), together with the deliberate departures from the
brief's wording.

This file is the index. It lists every record, current and superseded (a test fails if one is missing).

| ID | Decision | Status |
|---|---|---|
| [D0001](spec/decisions/0001-storage-postgresql.md) | PostgreSQL is the single store | Superseded by D0013 |
| [D0002](spec/decisions/0002-rust-service-stack.md) | tokio + axum + sqlx + rmcp + clap | Superseded by D0014 |
| [D0003](spec/decisions/0003-process-topology.md) | One `corvus` binary; MCP/CLI/watch use the database directly, hook and UI use HTTP | Superseded by D0015 |
| [D0004](spec/decisions/0004-wakeups-listen-notify.md) | Wake-ups via trigger-emitted `LISTEN/NOTIFY` with periodic re-check | Superseded by D0016 |
| [D0005](spec/decisions/0005-claim-concurrency-protocol.md) | Row locks in fixed order + `SKIP LOCKED` + partial unique indexes | Superseded by D0017 |
| [D0006](spec/decisions/0006-idempotency-keys.md) | Per-table `(creator, idempotency_key)` unique index | Accepted |
| [D0007](spec/decisions/0007-time-source.md) | Application-supplied time through an injectable clock | Accepted |
| [D0008](spec/decisions/0008-tauri-shell-role.md) | Tauri 2 is a native shell around the served dashboard | Superseded by D0018 |
| [D0009](spec/decisions/0009-dashboard-frontend.md) | Dashboard in Svelte 5 with tree fragments and a sink-free bundle | Superseded by D0019 |
| [D0010](spec/decisions/0010-append-only-enforcement.md) | Append-only enforced by triggers | Accepted |
| [D0011](spec/decisions/0011-work-source-abstraction.md) | One work-source engine; Linear and Jira are adapters; Linear ships in v1 | Accepted |
| [D0012](spec/decisions/0012-comment-tracking.md) | Comment tracking by order key + tie set | Accepted |
| [D0013](spec/decisions/0013-storage-sqlite.md) | SQLite, embedded in the binary, is the single store | Accepted |
| [D0014](spec/decisions/0014-rust-stack-rusqlite.md) | rusqlite with synchronous services; tokio + axum + rmcp + clap at the edges | Accepted |
| [D0015](spec/decisions/0015-process-topology-single-binary.md) | One binary; every local process opens the database; the HTTP server is optional | Accepted |
| [D0016](spec/decisions/0016-wakeups-hints-table.md) | Wake-ups via a trigger-written hints table, detected by `data_version` | Accepted |
| [D0017](spec/decisions/0017-single-writer-concurrency.md) | Every mutation is one short `BEGIN IMMEDIATE` transaction; unique indexes as the backstop | Accepted |
| [D0018](spec/decisions/0018-self-contained-desktop.md) | The Tauri 2 app is self-contained: core in-process, UI over IPC | Superseded by D0020 |
| [D0019](spec/decisions/0019-dashboard-frontend-two-carriers.md) | Svelte 5 dashboard: one bundle, two carriers, prebuilt into the binary | Superseded by D0020 |
| [D0020](spec/decisions/0020-desktop-only-dashboard.md) | The dashboard exists only in the desktop app; there is no browser UI | Accepted (spike pending — see Consequences) |
| [D0021](spec/decisions/0021-agent-adapters.md) | Agent tools are supported through declarative adapters; Codex is the first built-in | Accepted |
| [D0022](spec/decisions/0022-desktop-single-instance-local-socket.md) | The desktop app's single-instance guard may use a local OS rendezvous; it never opens a network listener | Accepted |
| [D0023](spec/decisions/0023-agent-identity-follows-conversation.md) | An agent's identity follows its tool conversation, so hooks need no per-project name | Accepted |
| [D0024](spec/decisions/0024-wake-through-the-tools-own-channel.md) | Idle sessions are woken through each tool's own channel; Corvus does not own the terminal | Accepted |
| [D0025](spec/decisions/0025-operator-reassigns-stranded-mail.md) | The operator can reassign an agent's unread mail; it is a transfer, not an acknowledgement | Accepted |

## Defaults and departures from the brief

See [spec/system/11-defaults.md](spec/system/11-defaults.md):

- [Defaults chosen where the brief is silent](spec/system/11-defaults.md#defaults) — numbered rows, each with
  its reason.
- [Deliberate departures from the brief's wording](spec/system/11-defaults.md#deliberate-departures-from-the-briefs-wording)
  — each tied to owner direction and a decision record.
