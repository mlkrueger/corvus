# Architecture

Corvus is a coordination service for the AI coding agents on one machine. It ships as **one self-contained
`corvus` binary** with SQLite compiled in, plus an optional Tauri desktop app that embeds the same core. The full
design is in [spec/system/](../spec/system/README.md); every choice below links to its decision record
([index](../DECISIONS.md)).

## In one picture

```
 agents (Claude Code, Codex, …)                 operator
   │ MCP stdio            │ hook              │ CLI          │ desktop app (dashboard over IPC)   scripts
   ▼                      ▼                   ▼              ▼                                   ▼ HTTP + token
 corvus mcp          corvus hook          corvus …        Corvus.app                         corvus serve
   └──────────────┬───────┴───────────────────┴──────────────┴───────────────────────────────────┘
                  ▼            every process opens the database itself (D0015)
            corvus-core services (Huginn: queue · Muninn: messages, memory · Hermod: wake-ups)
                  ▼            one BEGIN IMMEDIATE transaction per mutation (D0017)
            ~/.corvus/corvus.db   SQLite, WAL, synchronous=FULL (D0013)
```

## Layers (enforced)

| Crate | Role |
|---|---|
| `corvus-domain` | pure types and rules: names, ids, validation, presence, guidance text, filters, JQL/GraphQL variable builders, rich-text flattening, adapter templates. No IO (lint-checked). |
| `corvus-core` | config, clock, the one database opener and migrator, the store (SQL only), services (own every transaction), Hermod, background loops and leader lock, work-source engine, delivery helpers. The only crate that uses `rusqlite` (lint-checked). |
| `corvus-http` | the HTTP route table: an unauthenticated inner router used only in memory by the desktop app, and `serve()` which wraps it in host check → bearer auth → limits. |
| `corvus-mcp` | the MCP server: twenty tools, schemas from the core request structs. |
| `corvus-linear`, `corvus-jira` | read-only tracker adapters behind one `WorkSource` trait (D0011). |
| `corvus-cli` | the `corvus` binary: every subcommand, the hook fast path, stack commands. |
| `corvus-desktop` | Tauri 2 app: the core in-process, the dashboard over one IPC command, no listening port (D0020). |

## Key mechanisms

- **Atomic claiming** (P2): each claim is one `BEGIN IMMEDIATE` transaction that reaps expired leases and picks
  the best open, unblocked task; two partial unique indexes make a double claim impossible even for buggy code.
  Writers are serialised by SQLite's single write lock — the deliberate departure from "claimers never block
  each other" ([D0017](../spec/decisions/0017-single-writer-concurrency.md)). Measured hold time is ~1–2 ms.
- **Append-only history** (P4): triggers reject `UPDATE`/`DELETE` on messages, digests and events
  ([D0010](../spec/decisions/0010-append-only-enforcement.md)); read state lives in `receipts`.
- **Time** comes from an injectable clock bound as a query parameter, never SQL clock functions (lint-checked)
  ([D0007](../spec/decisions/0007-time-source.md)).
- **Wake-ups** (P5): triggers write targeted hints in the same transaction; each process watches
  `PRAGMA data_version` only while someone waits, and waiters always subscribe → read → wait → re-read, re-checking
  every 10 s ([D0016](../spec/decisions/0016-wakeups-hints-table.md)).
- **Idempotency** (P3): every create takes an optional key, unique per creator; replays return the original
  ([D0006](../spec/decisions/0006-idempotency-keys.md)).
- **Migrations** (P10): embedded, SHA-256 checksummed, applied in one locking transaction with a backup first; a
  newer database is refused.
- **Security** (P6, P7): see [spec/system/08](../spec/system/08-security.md). No network surface by default; the
  HTTP API exists only under `corvus serve` with a token; the dashboard renders all agent and tracker text as text
  (sink lint, built-bundle scan, strict CSP with Trusted Types, hostile-text browser tests).
- **Waking idle agents** goes through per-tool adapter files with a closed set of whole-argument placeholders and
  no way to pass message text ([D0021](../spec/decisions/0021-agent-adapters.md)).

## Known gaps

The authoritative list is kept in [spec/system/01 → Known gaps](../spec/system/01-architecture.md#known-gaps-kept-current-feeds-the-final-architecture-note).
In short:

- Writers are serialised (one SQLite write lock); claimers queue for one short transaction each.
- `CORVUS_DATABASE_URL` is optional (the brief says required) — Corvus creates its own database.
- Single machine, single OS user: anything running as that user can open the database file; the token protects
  only the HTTP surface. Agent names are claims, not identities.
- Cross-process wake-up latency is up to ~250 ms.
- Sessions auto-end and trackers sync only while `corvus serve` or the desktop app runs (claiming still reaps
  leases itself).
- The hook needs `CORVUS_AGENT_NAME`; duplicate delivery by the wake supervisor after a crash is possible.
- Tracker removals do not cancel tasks, and nothing is written back to Linear or Jira.
- No browser dashboard; a headless machine has the CLI and the HTTP API.
- The desktop app's single-instance guard uses a Unix-domain socket in `/tmp` (no network port).
- Not verified against real services or on other platforms: a real Claude Code session using the hook, Codex,
  Linear and Jira accounts, Linux desktop (WebKitGTK), macOS signing — see the plans' "Unverified" sections and
  the final report.
