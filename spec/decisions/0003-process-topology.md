# D0003 — One `corvus` binary; MCP/CLI/watch use the database directly, hook and UI use HTTP

- **Status:** Superseded by [D0015](0015-process-topology-single-binary.md)
- **Date:** 2026-11-13
- **Plans:** [0000](../plans/0000-roadmap.md), [0001](../plans/0001-foundation.md), [0004](../plans/0004-transports.md), [0007](../plans/0007-delivery-helpers.md)
- **Spec:** [system/01](../system/01-architecture.md#processes) · brief §5 ("all four wrap the same service layer")

## Problem

Corvus runs as several processes: the server, an MCP server per agent, CLI commands, wake supervisors, the
inbox hook, the dashboard. Each needs the same behaviour. Should every client go through the HTTP server, or
should processes embed the service layer and talk to PostgreSQL themselves? And how many binaries are there?

## Options

1. **Everything through HTTP.** MCP, CLI, watcher and hook are thin HTTP clients of `corvus serve`.
2. **Everything direct.** Every process, including the hook, links the service layer and opens the database.
3. **Hybrid.** Processes that are part of Corvus (MCP, CLI, watcher) embed the service layer; the tiny hook and
   the browser dashboard use HTTP.

## Trade-offs

### Everything through HTTP
- **For:** one process owns the database; one connection pool; clients are small.
- **Against:** the server becomes a single point of failure for agents — if it is down, nobody can claim or
  message; every operation is defined twice (route + client call); `corvus migrate` and `corvus serve` need the
  database anyway; contradicts the brief's "all four wrap the same service layer".

### Everything direct
- **For:** no HTTP dependency anywhere.
- **Against:** the hook must be "tiny, fast, failure-silent" — linking tokio and sqlx and opening a database
  connection on every tool call is the wrong shape; the dashboard is a web page and cannot do this at all.

### Hybrid
- **For:** agents keep working when the server is down; transports stay genuinely thin wrappers over one service
  layer; `LISTEN/NOTIFY` gives each process its own wake-ups ([D0004](0004-wakeups-listen-notify.md)); the hook
  stays a small blocking HTTP client.
- **Against:** several processes hold database connections (small pools needed); the database URL must reach the
  MCP process (via the launcher's env file); background loops run only in the server, so correctness must not
  depend on them.

## Decision

**Hybrid.** One `corvus` binary provides `serve`, `mcp`, `watch`, `migrate` and all CLI commands and links
`corvus-core`. `corvus-hook` is a separate minimal binary that calls the HTTP API. `corvus-desktop` is the Tauri
app. This matches the brief's wording, removes the server as a point of failure for agents, and keeps the hook
small.

## Consequences

- `claim_task` reaps expired leases itself, so the queue is correct even when the server's sweep is not running.
- Per-process pools are small (server 5; MCP/CLI/watch 2 plus one listener).
- A single binary means one install and one version; the hook is the only extra artifact agents need.
- Revisit if connection counts become a problem with many agents (then: MCP over HTTP as an option).

## Discussion

- 2026-11-13 — Initial record.
- 2026-11-13 — Re-decided for an embedded database: the hook no longer needs HTTP and scripts become subcommands. Superseded by D0015, which keeps the core conclusion (agents do not depend on the server).
