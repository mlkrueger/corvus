# Plans

One plan per feature or unit of work. Rules: [`/AGENTS.md` §3](../../AGENTS.md#3-plans). Template:
[`_template.md`](_template.md). Start with the [roadmap](0000-roadmap.md).

A plan says what gets built, in what order, and how it is verified. Lasting design goes in
[`../system/`](../system/README.md). Every plan ends with a `## Decisions` table that summarises and links the
[decision records](../decisions/README.md) it relies on.

## Index

| # | Plan | Status | Depends on |
|---|---|---|---|
| [0000](0000-roadmap.md) | Roadmap: build order and milestones | Ready | — |
| [0001](0001-foundation.md) | Foundation: workspace, config, embedded database, migrations | Done | — |
| [0002](0002-huginn-work-queue.md) | Huginn: sessions, tasks, claims, leases, cancellation | Done | 0001 |
| [0003](0003-muninn-hermod.md) | Muninn + Hermod: messaging, memory, export, wake-ups | Done | 0002 |
| [0004](0004-transports.md) | Transports: CLI, MCP server, HTTP API | Done | 0003 |
| [0005](0005-housekeeping.md) | Housekeeping loop and background leader | Done | 0004 |
| [0006](0006-dashboard-and-desktop.md) | Dashboard (Svelte) and self-contained desktop app (Tauri 2) | In progress | 0004, 0005 |
| [0007](0007-delivery-helpers.md) | Delivery helpers: hook, wake supervisor, agent adapters, stack commands | In progress | 0004 |
| [0008](0008-work-sources-and-linear.md) | Work-source engine + Linear adapter | Done | 0005, 0006 (panel) |
| [0009](0009-jira-source.md) | Jira adapter | Done | 0008 |
| [0010](0010-docs-and-release.md) | Docs, `DECISIONS.md`, full verification, honest report | In progress | all |
| [0011](0011-distribution.md) | Distribution: release binaries and the desktop bundle | In progress | 0006, 0007 |

Next free number: **0012**. Numbers are identifiers, not order — 0011 is built before 0010; see the roadmap.

Status values: `Draft` (being written) · `Ready` (agreed, not started) · `In progress` · `Done` · `Abandoned`.
