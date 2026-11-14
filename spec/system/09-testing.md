# 09 — Testing

Quality bar: brief §7. Rule: **anything about concurrency or atomicity is tested against real SQLite database
files — never mocks, never `:memory:`** (in-memory databases do not exercise WAL or multi-process locking).

The whole suite runs with `cargo test`: no services, containers or setup.

## Layers

| Layer | Where | Runs against | Tooling |
|---|---|---|---|
| Domain unit | `corvus-domain` | nothing (pure) | `cargo test`; property tests (`proptest`) for validators, JQL escaping, rich-text flattening |
| Service integration | `corvus-core/tests` | a fresh database file in a temp dir per test; manual `Clock` | `tempfile` |
| Multi-process | `corvus-cli/tests` | one database file, many spawned `corvus` processes | `assert_cmd`, barrier file |
| Transport | `corvus-http/tests`, `corvus-mcp/tests`, `corvus-cli/tests` | temp database + in-process router / spawned binary | `tower::ServiceExt::oneshot`, `assert_cmd` |
| Work sources | `corvus-jira/tests`, `corvus-linear/tests`, engine tests in core | temp database + an in-process fake tracker (axum on an ephemeral loopback port) | scripted pages, failures, comments |
| Helpers | hook and watcher tests | temp database; temp state dirs; stub commands | `assert_cmd`, `tempfile` |
| Dashboard unit | `ui/` | jsdom | `vitest` |
| Dashboard bundle | `ui/e2e` | the built bundle in WebKit and Chromium, with a test-only shim routing the `api` call to a throwaway `corvus serve` | Playwright |
| Desktop app | `corvus-desktop` | the `api` command, no-listener check, navigation guard, capabilities | WebDriver (`tauri-driver`) on Linux if workable; manual checklist on macOS |

## Commands

| Command | Does |
|---|---|
| `make test` | `cargo test --workspace` + `ui` unit tests + bundle scan |
| `make test-e2e` | dashboard bundle tests in browser engines; desktop window tests where automated |
| `make lint` | `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `ui` lint + typecheck, domain-purity check, "only `corvus-core::db` uses rusqlite" check, generated TypeScript types are current |

## Concurrency test technique

- **Threads**: N ≥ 16 threads, each with its *own* connection to one database file, released together with a
  `Barrier`; scenario repeated many times; assertions on the **database state**, not only return values.
- **Processes**: the same races with N spawned `corvus` processes started together — the guarantee is
  cross-process, so it is tested cross-process.
- Every race test has a timeout (a hang fails) and asserts no operation returned `Busy`.
- Stress tests record write-transaction durations and assert a ceiling, which catches IO sneaking into a
  write transaction.
- No test sleeps to wait for time to pass: time comes from the manual clock; timers use paused tokio time.

## Required test matrix

Every row must have at least one test that has actually been run. The owning plan lists the concrete cases.

| Area (brief §7) | Plan |
|---|---|
| Migrations ordered, checksummed, re-runnable, locked; edited migration rejected; newer database refused | [0001](../plans/0001-foundation.md) |
| Append-only enforcement (UPDATE / DELETE / bare DELETE rejected) | [0001](../plans/0001-foundation.md) |
| Concurrent claim races (threads and processes); no double claim | [0002](../plans/0002-huginn-work-queue.md) |
| Lease expiry; renew/release/complete ownership and replays | [0002](../plans/0002-huginn-work-queue.md) |
| Dependency blocking; cancellation, cascade, cancel-versus-claim race | [0002](../plans/0002-huginn-work-queue.md) |
| Idempotency replays for every create | [0002](../plans/0002-huginn-work-queue.md), [0003](../plans/0003-muninn-hermod.md) |
| Presence derivation; auto-end vs concurrent heartbeat | [0002](../plans/0002-huginn-work-queue.md), [0005](../plans/0005-housekeeping.md) |
| Unread / ack / unresolved-question semantics; direct-thread pairing | [0003](../plans/0003-muninn-hermod.md) |
| wait-for-inbox wake-ups (same and cross process), timeouts, lost-hint tolerance | [0003](../plans/0003-muninn-hermod.md) |
| Export contents and path safety | [0003](../plans/0003-muninn-hermod.md) |
| Auth on every HTTP route (none exempt); token compared safely; host allow-list; IPC seam; app opens no network listener | [0004](../plans/0004-transports.md), [0006](../plans/0006-dashboard-and-desktop.md) |
| Housekeeping isolation; background leader hand-over | [0005](../plans/0005-housekeeping.md) |
| Dashboard bundle has no data and no untrusted-markup sinks; hostile text renders literally | [0006](../plans/0006-dashboard-and-desktop.md) |
| Hook dedupe / throttle / stop-loop protection / never fails | [0007](../plans/0007-delivery-helpers.md) |
| Wake supervisor retry, backoff, dedupe, restart; adapter runner rules; built-in `codex` adapter | [0007](../plans/0007-delivery-helpers.md) |
| Source engine: assignment, idempotent re-poll, comment routing by task state, follow-ups, ignored accounts, re-assignment, rule scoping, manual queueing + its comment sync, partial-page abort, single flight | [0008](../plans/0008-work-sources-and-linear.md) |
| Linear adapter against a fake Linear; filter → variables; config validation | [0008](../plans/0008-work-sources-and-linear.md) |
| Jira adapter against a fake Jira; filter validation, query escaping, rich-text flattening, config validation | [0009](../plans/0009-jira-source.md) |
| The binary runs on a clean machine with nothing else installed | [0011](../plans/0011-distribution.md) |

The auth test enumerates the router's routes programmatically, so a newly added route cannot be forgotten.

## Honesty rule

The final report ([plan 0010](../plans/0010-docs-and-release.md)) lists what passes, what was not built and
what could not be verified. Real Jira and Linear accounts, and the real Claude Code / Codex clients, are
outside the automated suite; anything checked by hand is reported as checked by hand.
