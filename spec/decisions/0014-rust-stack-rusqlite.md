# D0014 — rusqlite with synchronous services; tokio + axum + rmcp + clap at the edges

- **Status:** Accepted
- **Date:** 2026-11-13
- **Supersedes:** [D0002](0002-rust-service-stack.md)
- **Plans:** [0001](../plans/0001-foundation.md), [0004](../plans/0004-transports.md)
- **Spec:** [system/01](../system/01-architecture.md#stack) · P9, P10

## Problem

With SQLite as the store ([D0013](0013-storage-sqlite.md)), the library choices made for Postgres need
revisiting. SQLite is an in-process, synchronous C library: there is no network round trip to await. How should
Rust talk to it, how are migrations run, and what does that do to the shape of the service layer?

## Options

Database access:
1. **rusqlite** (bundled SQLite) with synchronous services and a small in-house migrator.
2. **sqlx** with its SQLite driver — async API, compile-time-checked queries, built-in migrator.
3. **diesel** with SQLite.

HTTP (axum), MCP (rmcp), CLI (clap), outbound HTTP (reqwest + rustls) and logging (tracing) are unchanged from
D0002; their trade-offs are recorded there and still apply.

## Trade-offs

### rusqlite, synchronous services
- **For:** the direct, complete binding — `BEGIN IMMEDIATE`, pragmas, `data_version`, backup and `VACUUM INTO`
  are first-class; `bundled` compiles a known SQLite version into the binary, so features never depend on the
  OS; services become plain functions over a connection, trivially unit-testable and usable from the hook
  without an async runtime; no hidden worker threads.
- **Against:** no compile-time query checking — SQL errors surface in tests, so every query needs a test;
  async transports must hop to a blocking thread; the migrator is ours to write (about a hundred lines) and to
  get right.

### sqlx (SQLite)
- **For:** compile-time-checked SQL; migrator with checksums included; one API if the store ever changes.
- **Against:** wraps the synchronous library in an async facade with its own worker thread per connection;
  explicit transaction modes, pragmas and multi-process migration locking are less direct; pulls the async
  runtime into every code path, including the hook.

### diesel
- **For:** typed queries.
- **Against:** the DSL adds little for a schema this size and gets in the way of SQLite-specific statements.

## Decision

**rusqlite (bundled), synchronous services, in-house migrator.** The service layer is ordinary blocking Rust;
an async facade in `corvus-core` runs calls on the blocking pool for the tokio-based transports. Around it:
tokio, axum, rmcp, clap (derive), reqwest + rustls, tracing (JSON to stderr), serde, thiserror, uuid (v7),
subtle, sha2. Control over transaction mode and pragmas matters more here than compile-time SQL checking, and
synchronous services are what let one binary serve as CLI, MCP server and hook with fast start-up.

## Consequences

- Every SQL statement is exercised by at least one test (there is no compile-time safety net).
- The migrator must itself be tested for ordering, checksums, locking and refusal on a newer database
  ([system/02](../system/02-data-model.md#migrations)).
- Building needs a C compiler (for bundled SQLite); running needs nothing.
- If rmcp lags a needed protocol feature, the MCP crate is small and isolated enough to replace.

## Discussion

- 2026-11-13 — Created with the move to SQLite. Supersedes D0002.
- 2026-11-13 — Plan 0004 (rmcp 3.10.0, the current release; the plan noted training-era knowledge might be stale).
  `corvus-mcp` implements `ServerHandler` by hand (`get_info`, `list_tools`, `call_tool`) rather than with the
  `#[tool]` macros: that keeps the tool table explicit and lets each input schema be generated with
  `schemars::schema_for!` from the `corvus-core::api` request struct, then adjusted (the remembered agent and
  session become optional; the HTTP path id becomes a property). Many rmcp types are `#[non_exhaustive]` and are
  built with constructors (`Tool::new`, `CallToolResult::success/error`, `ListToolsResult::with_all_items`);
  `serve((reader, writer))` + `.waiting()` runs until the transport closes. The decision stands.
