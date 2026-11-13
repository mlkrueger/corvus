# D0002 — tokio + axum + sqlx + rmcp + clap

- **Status:** Superseded by [D0014](0014-rust-stack-rusqlite.md)
- **Date:** 2026-11-13
- **Plans:** [0001](../plans/0001-foundation.md), [0004](../plans/0004-transports.md)
- **Spec:** [system/01](../system/01-architecture.md#stack) · P9, P10

## Problem

The owner fixed the language (Rust) and the UI technology (Tauri 2). The service libraries still have to be
chosen: async runtime, HTTP framework, database access and migrations, MCP implementation, CLI parser, outbound
HTTP and logging. They must work together, and the database layer must make the locking SQL in
[D0005](0005-claim-concurrency-protocol.md) easy to write and hard to get wrong.

## Options

Database access:
1. **sqlx** — async, raw SQL, compile-time-checked queries, built-in migrator.
2. **diesel** (+ diesel-async) — typed query DSL.
3. **tokio-postgres** (+ refinery for migrations) — thin driver.

HTTP: **axum** vs **actix-web**. MCP: **rmcp** (official SDK) vs a hand-written JSON-RPC stdio loop.

## Trade-offs

### sqlx
- **For:** the design is SQL-first (`FOR UPDATE SKIP LOCKED`, partial-index `ON CONFLICT`, recursive CTEs) and
  sqlx runs that SQL verbatim while checking it against the schema at compile time; `sqlx::migrate!` already
  provides ordered, checksummed, advisory-locked, one-way migrations (P10); `PgListener` covers `LISTEN`;
  `#[sqlx::test]` gives a fresh database per test.
- **Against:** compile-time checking needs a database or committed offline metadata (`.sqlx/`); no query
  builder, so dynamic filters are hand-assembled.

### diesel
- **For:** strongest compile-time typing; mature.
- **Against:** the DSL fights the locking and conflict clauses this design depends on; async is an add-on;
  separate migration tooling.

### tokio-postgres + refinery
- **For:** minimal, transparent.
- **Against:** no compile-time checking; more hand-written mapping; two libraries to do what sqlx does in one.

### axum vs actix-web
- axum is tokio/tower-native, so auth, host allow-list, body limits and timeouts are ordinary tower layers, and
  the router can be tested in-process with `oneshot`. actix-web is equally capable but brings its own runtime
  conventions for no gain here.

### rmcp vs hand-written
- rmcp tracks the protocol, derives tool schemas from Rust types and handles stdio framing. Hand-writing saves a
  dependency but makes protocol drift our problem.

## Decision

**sqlx, axum, rmcp**, with tokio, clap (derive), reqwest + rustls, tracing (JSON to stderr), serde, thiserror,
uuid (v7), subtle. sqlx wins because the design is written in SQL and sqlx's migrator matches P10 exactly; axum
because its middleware model matches the security layers; rmcp because protocol conformance is not where this
project should spend effort.

## Consequences

- `.sqlx/` offline query metadata is committed so the workspace builds without a database; `make lint` checks it
  is current.
- Dynamic list filters use `sqlx::QueryBuilder` with bind parameters only.
- If rmcp lags a needed protocol feature, the MCP crate is small and isolated enough to replace.

## Discussion

- 2026-11-13 — Initial record. Versions are pinned by `Cargo.lock` at implementation, not here.
- 2026-11-13 — The database library choice followed from PostgreSQL. With SQLite the store is in-process and synchronous. Superseded by D0014.
