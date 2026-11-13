# Corvus spec — map

Everything needed to build Corvus. Rules for using and extending this tree are in [`/AGENTS.md`](../AGENTS.md).

```
spec/
  brief.md        WHAT  — frozen original requirements (cite as "brief §N", properties as P1..P10)
  system/         HOW   — the system spec; guides everything
  plans/          WHEN  — one plan per feature, in build order
  decisions/      WHY   — one record per significant decision
```

## I want to…

| Goal | Go to |
|---|---|
| Understand what Corvus must do | [brief.md](brief.md) |
| Understand the design in 5 minutes | [system/00-overview.md](system/00-overview.md), then [system/01-architecture.md](system/01-architecture.md) |
| Know what to build next | [plans/0000-roadmap.md](plans/0000-roadmap.md) |
| Know why something is the way it is | [decisions/README.md](decisions/README.md) |
| Find where a brief section is designed and planned | Traceability table below |
| Start a new feature | [plans/_template.md](plans/_template.md) |
| Record a choice | [decisions/_template.md](decisions/_template.md) |

## System spec

| # | File | Covers |
|---|---|---|
| 00 | [overview](system/00-overview.md) | Purpose, subsystems, properties → mechanisms, glossary |
| 01 | [architecture](system/01-architecture.md) | Stack, workspace layout, processes, layering |
| 02 | [data model](system/02-data-model.md) | Schema, constraints, triggers, migrations |
| 03 | [services](system/03-services.md) | Huginn / Muninn / Hermod behaviour, transactions, lock protocol |
| 04 | [interfaces](system/04-interfaces.md) | HTTP routes, MCP tools, CLI, error model |
| 05 | [dashboard](system/05-dashboard.md) | Svelte dashboard and the self-contained Tauri 2 app |
| 06 | [delivery helpers](system/06-delivery-helpers.md) | Inbox hook, wake supervisor, agent-tool adapters, stack commands |
| 07 | [work sources](system/07-work-sources.md) | Shared engine: filter, polling, reconciliation, HTTP surface |
| 07a | [jira](system/07a-jira.md) | Jira adapter |
| 07b | [linear](system/07b-linear.md) | Linear adapter |
| 08 | [security](system/08-security.md) | Threat model and controls |
| 09 | [testing](system/09-testing.md) | Test strategy and the required test matrix |
| 10 | [config & ops](system/10-config-ops.md) | Environment variables, logging, housekeeping |
| 11 | [defaults](system/11-defaults.md) | Small defaults, and deliberate departures from the brief's wording |

## Traceability

| Brief | System spec | Plan |
|---|---|---|
| §2 properties | [00](system/00-overview.md#properties--mechanisms) | all |
| §3 domain model | [02](system/02-data-model.md) | [0001](plans/0001-foundation.md) |
| §4.1 sessions, presence | [03](system/03-services.md#sessions-and-presence) | [0002](plans/0002-huginn-work-queue.md) |
| §4.2 tasks, claims, leases | [03](system/03-services.md#huginn-tasks-claims-leases) | [0002](plans/0002-huginn-work-queue.md) |
| §4.3 messaging, wake-ups | [03](system/03-services.md#muninn-messaging) | [0003](plans/0003-muninn-hermod.md) |
| §4.4 memory, export | [03](system/03-services.md#muninn-memory-and-export) | [0003](plans/0003-muninn-hermod.md) |
| §4.5 overview, stats, health | [03](system/03-services.md#overview-stats-health) | [0004](plans/0004-transports.md) |
| §4.6 agent guidance | [03](system/03-services.md#agent-guidance) | [0003](plans/0003-muninn-hermod.md) |
| §4.7 delivery helpers | [06](system/06-delivery-helpers.md) | [0007](plans/0007-delivery-helpers.md) |
| §4.8 housekeeping | [10](system/10-config-ops.md#housekeeping) | [0005](plans/0005-housekeeping.md) |
| §4.9 Jira | [07](system/07-work-sources.md), [07a](system/07a-jira.md) | [0008](plans/0008-work-sources-and-linear.md) (engine), [0009](plans/0009-jira-source.md) |
| Linear (owner addition) | [07](system/07-work-sources.md), [07b](system/07b-linear.md) | [0008](plans/0008-work-sources-and-linear.md) |
| §5 interfaces | [04](system/04-interfaces.md), [05](system/05-dashboard.md) | [0004](plans/0004-transports.md), [0006](plans/0006-dashboard-and-desktop.md) |
| §6 configuration | [10](system/10-config-ops.md) | [0001](plans/0001-foundation.md) |
| Self-contained binary, no containers (owner direction) | [01](system/01-architecture.md), [11](system/11-defaults.md#deliberate-departures-from-the-briefs-wording) | [0001](plans/0001-foundation.md), [0011](plans/0011-distribution.md) |
| §7 quality bar | [09](system/09-testing.md) | all, closed out in [0010](plans/0010-docs-and-release.md) |
