# Plan 0008 — Work-source engine + Linear adapter

- **Status:** Done
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

- [x] 1. `corvus-domain`: `SourceFilter` + validation, `TicketKey`, auto-queue rule matching, the comment
      "is new" function, task-body builder with the untrusted notice and truncation.
- [x] 2. Engine storage functions for `source_*` tables; `WorkSource` trait; a scripted in-memory fake source for tests.
- [x] 3. Sync pass: single-flight (in-process mutex + OS file lock), fetch-all-then-write (no network inside a
      write transaction), snapshot replace, sync state.
- [x] 4. Reconciliation: auto-queue, links, idempotent task creation, comment routing by task state,
      follow-ups, scope in/out, re-entry.
- [x] 5. Manual queueing; filter save (+ triggered sync); known-statuses cache.
- [x] 6. Routes `/api/sources…` (reachable over HTTP and desktop IPC); poll loop per enabled source inside the
      background leader, with error isolation; disabled-source behaviour.
- [x] 7. Verify Linear's GraphQL schema for the fields and filters in [07b](../system/07b-linear.md); correct the spec if needed.
- [x] 8. `corvus-linear`: config, constant query documents, filter → variables, pagination, mapping; fake Linear
      GraphQL server for tests; `corvus linear-sync`.
- [x] 9. Dashboard Sources panel (tabs, board, cards, actions, filter form with suggestions, sync status).

## Verification

Engine (temp SQLite database + scripted fake source) — this is the brief's Jira matrix, run source-agnostically:
- [x] assigned ticket matching a rule creates one task with title, mapped priority, pool, body incl. untrusted notice.
- [x] **idempotent re-poll:** identical pass → no new tasks, messages or events.
- [x] comment routing: open → appended to body; claimed → one message to the claimant in the task thread,
      marked untrusted; done/cancelled → follow-up task that becomes the live link.
- [x] task changes state between passes → comments follow the new state; a failure mid-delivery leaves the mark
      unmoved and the next pass delivers.
- [x] ignored accounts' comments are not delivered and do not re-trigger.
- [x] comment tie case: two comments with the same order key across two polls are both delivered once.
- [x] re-assignment: ticket leaves the result set → link out of scope; returns with task still live → resumes;
      returns after task finished → fresh task.
- [x] rule scoping by project; ticket in a shown status but not covered by a rule is on the board, not queued.
- [x] done-category tickets never auto-queue; appear in the "done in last N days" column data.
- [x] manual queueing: creates a `manual` task; conflict when a live task exists; allowed after it finishes;
      rejected for malformed keys, keys not on the board, done tickets; manual links survive scope changes and
      keep syncing comments while live.
- [x] **partial view:** failure on page 2 of search, or on any comment fetch → nothing written, `last_error` set.
- [x] **single flight:** a manual sync during a timer pass waits and then runs; two processes cannot interleave.
- [x] poll loop survives a failing source and keeps the process healthy; only the background leader polls.
- [x] a slow or hung tracker never holds the database write lock (other processes keep writing during a fetch).
- [x] filter validation: every bound and every malformed shape; filter never stores credentials.
- [x] routes: disabled source → `enabled:false` on read, 409 on mutations; unknown source name → 404; auth required.

Linear adapter (fake Linear server):
- [x] request contains the constant query text; statuses, team keys and cutoff appear only in `variables`,
      including hostile status names (quotes, braces, newlines rejected by validation; quotes pass safely).
- [x] pagination follows cursors to the end; GraphQL `errors`, HTTP 4xx/5xx and rate-limit responses fail the pass.
- [x] mapping: state type → category; priority map; `updatedAt` kept verbatim; non-`linear.app` URL dropped.
- [x] config: enabled only with the key; poll-seconds minimum; key never appears in logs or errors.
- [x] redirects are not followed; timeouts enforced.

Dashboard (the bundle harness from [plan 0006](0006-dashboard-and-desktop.md)): board columns, auto / by-hand badges, add and remove from queue, sync now, filter
form round trip with suggestions, error display; hostile ticket summary renders as text.

- [ ] By hand: against a real Linear workspace with a read-only key — issues appear, one is queued, a new
      comment reaches the task. (Not done — no Linear workspace available; see *Unverified*.)

## Exit criteria

- [x] `make lint`, `make test`, `make test-e2e` pass.
- [x] The engine tests use only the fake source — no Linear- or Jira-specific type appears in `corvus-core`.

## Open questions

- Exact Linear filter operators — settled at step 7 (recorded in [system/07b](../system/07b-linear.md)).
  Whether read-only personal API keys are available on the owner's workspace — still for the owner to check.
- Should completing a Corvus task ever update the Linear issue? Out of scope for v1; if wanted, it needs its own
  decision (it breaks the read-only property).

## Unverified / deferred

- **No real Linear workspace.** The adapter is tested against a fake GraphQL server and the confirmed published
  schema; real responses (rate-limit bodies, `or` + `isMe` results) are unexercised.
- The dashboard's Sources e2e tests use canned source data in the test shim (real trackers cannot be pointed at
  a fake from the browser harness); engine behaviour is covered by the Rust suites.
- The 10-minute `known_statuses` cache has no dedicated test.
- A database error part-way through reconciliation leaves the snapshot replaced but later tickets unreconciled
  until the next pass (each ticket is its own transaction, as specified); not tested.

## Notes from implementation

- Found by the engine test matrix and fixed: re-queueing an unchanged ticket after its task finished returned
  the old task (idempotency key collision), and a same-order-key comment on a finished task was never delivered
  (follow-up key collision). Keys are now as in [system/07](../system/07-work-sources.md#reconciliation).
- Manual links follow comments only while the task is live and the ticket is not done (system/07 step 6).
- `POST /api/sources/{source}/sync` reports a failed pass in its body (`{ok:false, error}`) and in `last_error`,
  not as an HTTP error.
- `corvus serve` and the desktop app build sources from config via `corvus_linear::from_config` /
  `corvus_jira::from_config`; poll loops run only in the background leader.
- End-to-end engine tests through the real Linear adapter + fake GraphQL server: `crates/corvus-linear/tests/end_to_end.rs`.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0011 | One source-agnostic engine with Linear and Jira as thin adapters; Linear ships in v1 and is built first, by owner direction. | [0011](../decisions/0011-work-source-abstraction.md) |
| D0012 | New comments are detected by an adapter-supplied order key plus the ids already seen at that key — exact for Linear's UUID ids and identical to the brief's numeric mark for Jira. | [0012](../decisions/0012-comment-tracking.md) |
