# System spec

The design of Corvus: HOW the requirements in [`../brief.md`](../brief.md) are met as one self-contained Rust
binary (embedded SQLite) plus a self-contained Tauri 2 desktop app.
This is the source of truth for design. Plans implement it; decisions justify it.

Read in order the first time; afterwards jump by topic.

| # | File | Covers |
|---|---|---|
| 00 | [overview](00-overview.md) | Purpose, subsystems, properties → mechanisms, glossary |
| 01 | [architecture](01-architecture.md) | Stack, workspace layout, processes, layering |
| 02 | [data model](02-data-model.md) | Schema, constraints, triggers, migrations |
| 03 | [services](03-services.md) | Huginn / Muninn / Hermod behaviour, transactions, lock protocol |
| 04 | [interfaces](04-interfaces.md) | HTTP routes, MCP tools, CLI, error model |
| 05 | [dashboard](05-dashboard.md) | Svelte dashboard and the self-contained Tauri 2 app |
| 06 | [delivery helpers](06-delivery-helpers.md) | Inbox hook, wake supervisor, agent-tool adapters, stack commands |
| 07 | [work sources](07-work-sources.md) | Shared engine: filter, polling, reconciliation, HTTP surface |
| 07a | [jira](07a-jira.md) | Jira adapter |
| 07b | [linear](07b-linear.md) | Linear adapter |
| 08 | [security](08-security.md) | Threat model and controls |
| 09 | [testing](09-testing.md) | Test strategy and the required test matrix |
| 10 | [config & ops](10-config-ops.md) | Environment variables, logging, housekeeping |
| 11 | [defaults](11-defaults.md) | Small defaults, and deliberate departures from the brief's wording |

Conventions: `brief §N` and `P1`..`P10` cite the brief; `D00NN` cites a [decision](../decisions/README.md).
Behaviour the brief already states precisely is **not restated** here — this spec adds the mechanism.
