# Plan 0009 — Jira adapter

- **Status:** Ready
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

- [ ] 1. Verify the current Jira Cloud v3 endpoints (enhanced JQL search paging, comment paging, statuses).
- [ ] 2. `corvus-domain`: JQL builder with escaping; `adf_to_text`; Jira priority map.
- [ ] 3. `corvus-jira`: config validation (all-or-nothing, https only), Basic auth, client with the outbound
      rules, pagination, mapping to `Ticket` / `Comment`.
- [ ] 4. Fake Jira server for tests (search pages, comments, statuses, failure injection).
- [ ] 5. Register the adapter; `corvus jira-sync`; default filter; Jira tab wording.
- [ ] 6. Re-run the engine matrix from 0008 through the Jira adapter + fake Jira as an end-to-end set.

## Verification

- [ ] config: only some variables set → error naming the missing ones; `http://` base URL rejected; none set →
      disabled; token never in logs, errors or the database.
- [ ] JQL: always contains `assignee = currentUser()`; statuses quoted and escaped — property test that no
      valid status name can change the query's structure; projects and done window clauses appear only when set.
- [ ] `adf_to_text`: paragraphs, headings, nested lists, code blocks, links, mentions, tables, hard breaks,
      unknown nodes, non-document input; property test that it never panics.
- [ ] client: follows `nextPageToken` to the last page; page failure fails the search; comment paging; redirect
      responses are not followed; timeout enforced; issue keys validated before entering a URL path.
- [ ] mapping: priority names; status category keys; `updated` kept verbatim (two stamps differing only in
      time-zone notation are treated as different, and nothing is parsed); numeric comment ids → order keys.
- [ ] end-to-end through the fake Jira: assignment, idempotent re-poll, comment routing by task state,
      follow-ups, ignored accounts, re-assignment, rule scoping by project, manual queueing and its comment sync.
- [ ] dashboard: the Jira tab shows the default filter (project AA rule); ticket keys are links only for
      `https:` URLs and open in the system browser, never in the app's webview.
- [ ] By hand (if a Jira site is available): real sync. Otherwise reported as not verified.

## Exit criteria

- [ ] `make lint`, `make test`, `make test-e2e` pass.
- [ ] `corvus-core` is unchanged by this plan except adapter registration.

## Open questions

- None beyond step 1's endpoint confirmation.

## Unverified / deferred

—

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0011 | Jira is an adapter on the shared work-source engine; routes are `/api/sources/jira/…` and tables are `source_*`. | [0011](../decisions/0011-work-source-abstraction.md) |
| D0012 | Jira's numeric comment id is the order key, which reduces the engine's mechanism to exactly the brief's numeric high-water mark. | [0012](../decisions/0012-comment-tracking.md) |
