# 00 — Overview

## Purpose

Corvus coordinates AI coding-agent sessions on one developer machine (brief §1). Agents pull; Corvus never pushes
into a running agent. Its job is to make pulling reliable and cheap.

It ships as **one self-contained Rust binary** with an embedded database, and an optional self-contained
desktop app. Nothing else needs to be installed or running.

## Subsystems

| Name | Role | Spec |
|---|---|---|
| **Huginn** | Work queue: tasks, claims, leases, dependencies, cancellation | [03](03-services.md#huginn-tasks-claims-leases) |
| **Muninn** | Memory and messaging: threads, messages, receipts, digests, decisions, audit events | [03](03-services.md#muninn-messaging) |
| **Hermod** | Wake-ups: block until something relevant happens | [03](03-services.md#hermod-wake-ups) |
| Transports | HTTP API, MCP server (stdio), CLI, desktop IPC | [04](04-interfaces.md) |
| Dashboard | Operator UI: the self-contained Tauri 2 desktop app (Svelte frontend over IPC). No browser UI. | [05](05-dashboard.md) |
| Delivery helpers | Claude Code inbox hook, wake supervisor, agent-tool adapters (Codex built in), stack commands | [06](06-delivery-helpers.md) |
| Work sources | Read-only feeds turning **Linear** and **Jira** tickets into tasks | [07](07-work-sources.md), [07a](07a-jira.md), [07b](07b-linear.md) |

## Shape in one picture

```
   the `corvus` binary                                   Corvus desktop app (Tauri 2)
 ┌───────────────────────────────┐                     ┌──────────────────────────────┐
 │ corvus mcp    (per agent)     │                     │ Svelte dashboard             │
 │ corvus <cmd>  (operator CLI)  │                     │      │ IPC                   │
 │ corvus hook   (read-only)     │                     │ in-memory API router         │
 │ corvus watch  (wake superv.)  │                     │ corvus-core                  │
 │ corvus serve  (HTTP + loops)  │                     │ background loops (if leader) │
 └───────────────┬───────────────┘                     └───────────────┬──────────────┘
                 │        every process opens the file directly        │
                 └───────────────────────┬─────────────────────────────┘
                                         ▼
                           ~/.corvus/corvus.db   (SQLite, WAL)
                           the one source of truth; hints table for wake-ups

 scripts / other tools ──HTTP + token──► corvus serve                           [optional]
 background leader ──HTTPS──► Linear, Jira Cloud (read-only)                     [optional]
```

Agents need only the binary and the file. The HTTP API, the dashboard, tracker polling and background
housekeeping are optional layers on top ([D0015](../decisions/0015-process-topology-single-binary.md)).

## Properties → mechanisms

Each non-negotiable property (brief §2) and the mechanism that guarantees it.

| | Property | Mechanism | Detail |
|---|---|---|---|
| P1 | One durable source of truth | One SQLite file (WAL, `synchronous = FULL`); exports are write-only snapshots | [D0013](../decisions/0013-storage-sqlite.md), [02](02-data-model.md) |
| P2 | Atomic claiming | Every mutation is one `BEGIN IMMEDIATE` transaction; partial unique indexes as backstop | [D0017](../decisions/0017-single-writer-concurrency.md), [03](03-services.md#lock-protocol) |
| P3 | Idempotent mutations | `(creator, idempotency_key)` partial unique index per creatable table + `ON CONFLICT DO NOTHING` | [D0006](../decisions/0006-idempotency-keys.md) |
| P4 | Append-only history | `BEFORE UPDATE` / `BEFORE DELETE` triggers that raise | [D0010](../decisions/0010-append-only-enforcement.md) |
| P5 | Notifications are hints | Trigger-written `hints` table, detected via `data_version`; subscribe-then-read; periodic re-check | [D0016](../decisions/0016-wakeups-hints-table.md) |
| P6 | Secure by default | No listener unless a token is set; then loopback bind, bearer token, host allow-list; env-only secrets | [08](08-security.md) |
| P7 | Untrusted text stays text | Text-node rendering, strict CSP, bundle scan, single-capability webview, untrusted labels | [D0020](../decisions/0020-desktop-only-dashboard.md), [05](05-dashboard.md) |
| P8 | Never fatal in the background | Per-step error isolation in housekeeping and source loops | [10](10-config-ops.md#housekeeping) |
| P9 | Observable | `tracing` JSON lines to stderr; audit event in every mutating transaction | [10](10-config-ops.md#logging) |
| P10 | Versioned schema | Embedded migrations: ordered, SHA-256 checksummed, one-way, run inside one write transaction | [02](02-data-model.md#migrations) |

## Glossary

| Term | Meaning |
|---|---|
| Agent | A named participant (`claude_code`, `codex`, `operator`, `jira`, `linear`). Created on first use. |
| Session | One live connection of an agent; UUID-identified; several may share an agent name. |
| Claim | A session's time-limited hold (lease) on a task. |
| Hint | A wake-up notification. Says "go look", carries no state. |
| Operator | The human. Posts as `CORVUS_OPERATOR_NAME` (default `operator`). |
| Bootstrap context | What an agent needs on connect: assigned tasks, open tasks, unread, unresolved questions, digests, decisions. |
| Guidance | The single operating-instruction text shared by MCP, `connect`, the hook and the docs. |
| `api` command | The single IPC command through which the desktop dashboard calls the route table in memory. |
| Background leader | The one long-lived process currently running housekeeping and tracker polling. |
