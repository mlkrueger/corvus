# D0011 — One work-source engine; Linear and Jira are adapters; Linear ships in v1

- **Status:** Accepted
- **Date:** 2026-11-13
- **Plans:** [0000](../plans/0000-roadmap.md), [0008](../plans/0008-work-sources-and-linear.md), [0009](../plans/0009-jira-source.md)
- **Spec:** [system/07](../system/07-work-sources.md) · [07a](../system/07a-jira.md) · [07b](../system/07b-linear.md) · brief §4.9

## Problem

The brief specifies a Jira source in detail (§4.9). The owner added a requirement while this spec was being
written: **a Linear integration from the beginning.** The brief says nothing about Linear. How should a second
tracker be added — and what should it do — without doubling the most intricate part of the system (the
reconciliation rules for snapshots, links, comments, scope and manual queueing)?

## Options

1. **Two independent integrations** — build Jira per the brief, then write Linear separately with its own
   tables, routes and reconciliation.
2. **One engine, two adapters** — generalise §4.9 into a source-agnostic engine; each tracker implements a small
   `WorkSource` trait (search, comments, known statuses).
3. **Linear instead of Jira** — replace the Jira source.
4. **Jira now, Linear later** — keep v1 to the brief and add Linear in a follow-up.

## Trade-offs

### Two independent integrations
- **For:** each can follow its tracker's quirks freely; the Jira code matches the brief word for word.
- **Against:** two copies of the hardest logic and of its test matrix; behaviours drift; two dashboard panels,
  two sets of routes and tables.

### One engine, two adapters
- **For:** reconciliation is written and tested once; adapters are thin, pure translators that are easy to test
  against a fake; the dashboard panel, filter format and routes are shared; a third tracker later is an adapter.
- **Against:** the abstraction must be right the first time — Linear differs in real ways (GraphQL, teams
  instead of projects, UUID comment ids, Markdown instead of ADF); Jira table and route names become
  `source_*` / `/api/sources/jira` rather than literally "Jira routes".

### Linear instead of Jira
- **For:** least work.
- **Against:** drops a brief requirement and its required tests; the owner asked for Linear *as well*.

### Jira now, Linear later
- **For:** v1 scope equals the brief.
- **Against:** contradicts the owner's direction; retrofitting an abstraction after Jira-specific tables and
  routes exist costs more than designing it in.

## Decision

**Option 2.** A shared engine in `corvus-core` with `corvus-linear` and `corvus-jira` adapters. Linear's
behaviour is defined as "§4.9 with Linear vocabulary": same filter shape, same reconciliation, same manual
queueing, read-only. The differences are confined to the adapter ([07b](../system/07b-linear.md)).
**Build order: engine + Linear first, then Jira**, because the owner asked for Linear at the start; the engine's
test matrix is written against a scripted fake source so it does not depend on either tracker.

## Consequences

- Tables are `source_*` keyed by `source`; routes are `/api/sources/{source}/…`; the dashboard has one Sources
  panel with a tab per enabled source.
- Comment tracking had to be generalised beyond a numeric id: [D0012](0012-comment-tracking.md).
- Both sources are strictly read-only in v1. Writing status back to Linear or Jira (for example moving an issue
  when a task completes) is a separate future decision.
- Swapping the build order (Jira before Linear) only swaps plans 0008 and 0009's adapter halves.

## Discussion

- 2026-11-13 — Initial record, prompted by the owner's mid-session request for Linear. Interpretation to
  confirm: "at the beginning" was read as "in v1, built before Jira", and as a work-queue feed like Jira — not
  as publishing Corvus tasks into Linear.
- 2026-11-13 — Storage moved to SQLite (D0013); no effect on this decision. Sync single-flight uses an OS file lock instead of a Postgres advisory lock.
