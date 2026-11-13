# Plan 0009 — Jira adapter

- **Status:** Done
- **Depends on:** [0008](0008-work-sources-and-linear.md)
- **Spec:** [system/07a](../system/07a-jira.md) · [system/07](../system/07-work-sources.md) · brief §4.9

## Goal

The Jira Cloud source works exactly as brief §4.9 describes, implemented as a second adapter on the engine from
plan 0008 with no engine changes.

## Scope

**In:** `corvus-jira` (config, REST client, JQL builder, ADF flattening, mapping), `corvus jira-sync`, the Jira
tab's defaults and labels, the brief's Jira-specific tests.

**Out:** engine behaviour (done and tested in 0008); inbound webhooks (brief: none); writing to Jira (never).

## Work breakdown

- [x] 1. Verify the current Jira Cloud v3 endpoints (enhanced JQL search paging, comment paging, statuses).
- [x] 2. `corvus-domain`: JQL builder with escaping; `adf_to_text`; Jira priority map.
- [x] 3. `corvus-jira`: config validation (all-or-nothing, https only), Basic auth, client with the outbound
      rules, pagination, mapping to `Ticket` / `Comment`.
- [x] 4. Fake Jira server for tests (search pages, comments, statuses, failure injection).
- [x] 5. Register the adapter; `corvus jira-sync`; default filter; Jira tab wording.
- [x] 6. Re-run the engine matrix from 0008 through the Jira adapter + fake Jira as an end-to-end set.

## Verification

- [x] config: only some variables set → error naming the missing ones; `http://` base URL rejected; none set →
      disabled; token never in logs, errors or the database.
- [x] JQL: always contains `assignee = currentUser()`; statuses quoted and escaped — property test that no
      valid status name can change the query's structure; projects and done window clauses appear only when set.
- [x] `adf_to_text`: paragraphs, headings, nested lists, code blocks, links, mentions, tables, hard breaks,
      unknown nodes, non-document input; property test that it never panics.
- [x] client: follows `nextPageToken` to the last page; page failure fails the search; comment paging; redirect
      responses are not followed; timeout enforced; issue keys validated before entering a URL path.
- [x] mapping: priority names; status category keys; `updated` kept verbatim (two stamps differing only in
      time-zone notation are treated as different, and nothing is parsed); numeric comment ids → order keys.
- [x] end-to-end through the fake Jira: assignment, idempotent re-poll, comment routing by task state,
      follow-ups, ignored accounts, re-assignment, rule scoping by project, manual queueing and its comment sync.
- [x] dashboard: the Jira tab shows the default filter (project AA rule); ticket keys are links only for
      `https:` URLs and open in the system browser, never in the app's webview.
- [ ] By hand (if a Jira site is available): real sync. **Not verified — no Jira site available.**

## Exit criteria

- [x] `make lint`, `make test`, `make test-e2e` pass.
- [x] `corvus-core` is unchanged by this plan except adapter registration.

## Open questions

- None beyond step 1's endpoint confirmation.

## Unverified / deferred

- **No real Jira site**: everything runs against a fake Jira. Endpoint shapes were confirmed against Atlassian's
  OpenAPI document (recorded in [system/07a](../system/07a-jira.md#api-usage-rest-v3-read-only)).
- Clicking a ticket link in the real desktop window (system browser hand-off) is on plan 0006's manual checklist.

## Notes from implementation

- Search pages are 50 issues (Jira's cap). Paging guards: at most 1,000 pages; a repeated `nextPageToken` fails
  the pass. `adf_to_text` stops descending at depth 40.
- Basic auth is encoded by a small in-crate base64 function (no extra dependency).
- `corvus-core` changed only for registration-independent fixes found by the 0008 engine tests; the adapter is
  wired by the binaries (`corvus_jira::from_config`).
- End-to-end engine matrix through the real adapter + fake Jira: `crates/corvus-jira/tests/end_to_end.rs`.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0011 | Jira is an adapter on the shared work-source engine; routes are `/api/sources/jira/…` and tables are `source_*`. | [0011](../decisions/0011-work-source-abstraction.md) |
| D0012 | Jira's numeric comment id is the order key, which reduces the engine's mechanism to exactly the brief's numeric high-water mark. | [0012](../decisions/0012-comment-tracking.md) |
