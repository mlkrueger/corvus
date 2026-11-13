# D0015 — One binary; every local process opens the database; the HTTP server is optional

- **Status:** Accepted
- **Date:** 2026-11-13
- **Supersedes:** [D0003](0003-process-topology.md)
- **Plans:** [0000](../plans/0000-roadmap.md), [0001](../plans/0001-foundation.md), [0004](../plans/0004-transports.md), [0007](../plans/0007-delivery-helpers.md), [0011](../plans/0011-distribution.md)
- **Spec:** [system/01](../system/01-architecture.md#processes) · brief §5

## Problem

Corvus must ship as a self-contained binary ([D0013](0013-storage-sqlite.md)). Several processes need the same
behaviour: an MCP server per agent, CLI commands, the inbox hook, wake supervisors, the desktop app, and an HTTP
API. With an embedded database, who opens it? Is there a daemon that must be running for agents to work? And
how many artifacts does a user install?

## Options

1. **Single-owner daemon.** One long-running process owns the database; MCP, CLI, hook and UI are clients of
   its loopback HTTP API.
2. **Shared file.** Every local Corvus process opens the SQLite file directly; a daemon exists only for the
   HTTP API and background loops.
3. **Shared file with auto-spawned daemon** — as 1, but clients start the daemon on demand.

## Trade-offs

### Single-owner daemon
- **For:** one process, one writer — the simplest concurrency story; wake-ups are in-process and instant.
- **Against:** nothing works unless the daemon is up — the most common failure for tools of this kind ("MCP
  server failed: connection refused"); every operation is implemented twice (route and client); the token must
  reach every agent process; contradicts "all four wrap the same service layer".

### Shared file
- **For:** agents need **no running service at all**: `corvus mcp` opens the file and works; no token or port
  in the agent path; the HTTP listener becomes opt-in, so a default install has no network surface; transports
  stay thin wrappers over one service layer; the hook can read the database directly, faster than HTTP.
- **Against:** multi-process SQLite must be done carefully (WAL, one write transaction mode, busy handling,
  version-skew checks); cross-process wake-ups need a mechanism ([D0016](0016-wakeups-hints-table.md));
  background loops (housekeeping, tracker polling) only run while a long-lived process is up, so correctness
  must not depend on them; two long-lived processes need a leader rule.

### Auto-spawned daemon
- **For:** hides the daemon most of the time.
- **Against:** all of option 1's coupling plus spawn races, orphaned daemons and upgrade skew between a running
  daemon and new clients.

## Decision

**Shared file.** There is **one `corvus` binary**: CLI, `mcp`, `hook`, `watch`, `serve`, `up`/`down`/`status`.
Each invocation opens the database itself. `corvus serve` (and the desktop app) additionally run the background
loops and, when a token is configured, the HTTP API. The inbox hook is a subcommand that reads the database
directly — there is no separate hook binary and no shell scripts to ship.

## Consequences

- `claim_task` reaps expired leases itself; queue correctness never depends on a background loop.
- Exactly one process runs the background loops at a time, chosen by an OS file lock
  ([system/10](../system/10-config-ops.md#background-leader)).
- Every writing process verifies the schema version and migrates on open; a binary older than the database
  refuses to run ([system/02](../system/02-data-model.md#migrations)).
- The brief's stack scripts and MCP launcher script become subcommands and built-in env-file loading; recorded
  in [defaults](../system/11-defaults.md).
- A second artifact, the desktop app, embeds the same core ([D0020](0020-desktop-only-dashboard.md)).

## Discussion

- 2026-11-13 — Created with the move to a self-contained binary. Supersedes D0003; keeps its conclusion that
  agents must not depend on the server, and goes further by removing the hook's dependency too.
- 2026-11-13 — Narrowed by D0020: only `corvus serve` hosts the HTTP API. The desktop app runs the background loops but never opens a listener.
