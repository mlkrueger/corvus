# Plan 0008 — Work-source engine + Linear adapter

- **Status:** Ready
- **Depends on:** [0005](0005-housekeeping.md) (background leader); the panel step needs [0006](0006-dashboard-and-desktop.md)
- **Spec:** [system/07](../system/07-work-sources.md) · [system/07b](../system/07b-linear.md) · brief §4.9 (generalised)

## Goal

Linear issues assigned to the operator appear on a dashboard board and flow into the Huginn queue under
operator-defined rules, through a source-agnostic engine that the Jira adapter will reuse unchanged.

## Scope

**In:** the `WorkSource` trait and engine (filter, single-flight sync, reconciliation, manual queueing), source
HTTP routes, poll loop, `corvus linear-sync`, the `corvus-linear` adapter, the dashboard Sources panel.

**Out:** the Jira adapter ([0009](0009-jira-source.md)); writing anything back to Linear (future decision);
Linear webhooks (not planned — polling only, like Jira).

## Work breakdown

- [ ] 1. `corvus-domain`: `SourceFilter` + validation, `TicketKey`, auto-queue rule matching, the comment
      "is new" function, task-body builder with the untrusted notice and truncation.
- [ ] 2. Engine storage functions for `source_*` tables; `WorkSource` trait; a scripted in-memory fake source for tests.
- [ ] 3. Sync pass: single-flight (in-process mutex + OS file lock), fetch-all-then-write (no network inside a
      write transaction), snapshot replace, sync state.
- [ ] 4. Reconciliation: auto-queue, links, idempotent task creation, comment routing by task state,
      follow-ups, scope in/out, re-entry.
- [ ] 5. Manual queueing; filter save (+ triggered sync); known-statuses cache.
- [ ] 6. Routes `/api/sources…` (reachable over HTTP and desktop IPC); poll loop per enabled source inside the
      background leader, with error isolation; disabled-source behaviour.
- [ ] 7. Verify Linear's GraphQL schema for the fields and filters in [07b](../system/07b-linear.md); correct the spec if needed.
- [ ] 8. `corvus-linear`: config, constant query documents, filter → variables, pagination, mapping; fake Linear
      GraphQL server for tests; `corvus linear-sync`.
- [ ] 9. Dashboard Sources panel (tabs, board, cards, actions, filter form with suggestions, sync status).

## Verification

Engine (temp SQLite database + scripted fake source) — this is the brief's Jira matrix, run source-agnostically:
- [ ] assigned ticket matching a rule creates one task with title, mapped priority, pool, body incl. untrusted notice.
- [ ] **idempotent re-poll:** identical pass → no new tasks, messages or events.
- [ ] comment routing: open → appended to body; claimed → one message to the claimant in the task thread,
      marked untrusted; done/cancelled → follow-up task that becomes the live link.
- [ ] task changes state between passes → comments follow the new state; a failure mid-delivery leaves the mark
      unmoved and the next pass delivers.
- [ ] ignored accounts' comments are not delivered and do not re-trigger.
- [ ] comment tie case: two comments with the same order key across two polls are both delivered once.
- [ ] re-assignment: ticket leaves the result set → link out of scope; returns with task still live → resumes;
      returns after task finished → fresh task.
- [ ] rule scoping by project; ticket in a shown status but not covered by a rule is on the board, not queued.
- [ ] done-category tickets never auto-queue; appear in the "done in last N days" column data.
- [ ] manual queueing: creates a `manual` task; conflict when a live task exists; allowed after it finishes;
      rejected for malformed keys, keys not on the board, done tickets; manual links survive scope changes and
      keep syncing comments while live.
- [ ] **partial view:** failure on page 2 of search, or on any comment fetch → nothing written, `last_error` set.
- [ ] **single flight:** a manual sync during a timer pass waits and then runs; two processes cannot interleave.
- [ ] poll loop survives a failing source and keeps the process healthy; only the background leader polls.
- [ ] a slow or hung tracker never holds the database write lock (other processes keep writing during a fetch).
- [ ] filter validation: every bound and every malformed shape; filter never stores credentials.
- [ ] routes: disabled source → `enabled:false` on read, 409 on mutations; unknown source name → 404; auth required.

Linear adapter (fake Linear server):
- [ ] request contains the constant query text; statuses, team keys and cutoff appear only in `variables`,
      including hostile status names (quotes, braces, newlines rejected by validation; quotes pass safely).
- [ ] pagination follows cursors to the end; GraphQL `errors`, HTTP 4xx/5xx and rate-limit responses fail the pass.
- [ ] mapping: state type → category; priority map; `updatedAt` kept verbatim; non-`linear.app` URL dropped.
- [ ] config: enabled only with the key; poll-seconds minimum; key never appears in logs or errors.
- [ ] redirects are not followed; timeouts enforced.

Dashboard (the bundle harness from [plan 0006](0006-dashboard-and-desktop.md)): board columns, auto / by-hand badges, add and remove from queue, sync now, filter
form round trip with suggestions, error display; hostile ticket summary renders as text.

- [ ] By hand: against a real Linear workspace with a read-only key — issues appear, one is queued, a new
      comment reaches the task. Recorded below.

## Exit criteria

- [ ] `make lint`, `make test`, `make test-e2e` pass.
- [ ] The engine tests use only the fake source — no Linear- or Jira-specific type appears in `corvus-core`.

## Open questions

- Exact Linear filter operators (`isMe`, `completedAt` comparators) and whether read-only personal API keys are
  available on the owner's workspace — settled at step 7.
- Should completing a Corvus task ever update the Linear issue? Out of scope for v1; if wanted, it needs its own
  decision (it breaks the read-only property).

## Unverified / deferred

—

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0011 | One source-agnostic engine with Linear and Jira as thin adapters; Linear ships in v1 and is built first, by owner direction. | [0011](../decisions/0011-work-source-abstraction.md) |
| D0012 | New comments are detected by an adapter-supplied order key plus the ids already seen at that key — exact for Linear's UUID ids and identical to the brief's numeric mark for Jira. | [0012](../decisions/0012-comment-tracking.md) |
