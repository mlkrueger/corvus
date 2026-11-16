# Final report (plan 0010)

Built on 2026-11-13 on macOS 26 (Apple silicon), Rust 1.98.1, Node 26; owner checks and distribution finished
2026-11-14 to 2026-11-16. Everything below was run; nothing is
claimed that was not exercised. Plan-by-plan detail lives in each plan's "Unverified / deferred" section under
[spec/plans/](../spec/plans/README.md).

## What passes

| Gate | Result |
|---|---|
| `make lint` | pass — `cargo fmt --check`, `clippy -D warnings`, domain purity, rusqlite confinement, no SQL clock functions, HTTP seam, generated-type drift, CLI free of desktop deps, frontend ESLint + sink scan + typecheck |
| `make test` | pass — **497 Rust tests** (0 failed, 0 ignored), **114 dashboard unit tests**, built-bundle sink scan |
| `make test-e2e` | pass — **46 Playwright tests** (WebKit + Chromium) against a real `corvus serve`, plus the desktop app's no-listening-socket test with a real launch |
| Race suite, 50 consecutive runs | 50/50 (release build, core and CLI race files in parallel) |

### The brief's quality bar (system/09 required matrix) → tests

| Area | Where (all passing) |
|---|---|
| Migrations ordered, checksummed, re-runnable, locked; edited migration rejected; newer DB refused | `corvus-core/tests/migrations.rs`, `corvus-cli/tests/foundation.rs` |
| Append-only enforcement | `corvus-core/tests/append_only.rs`, `muninn.rs` |
| Concurrent claim races, threads and processes; no double claim | `corvus-core/tests/races.rs`, `corvus-cli/tests/races.rs`, `constraints.rs` |
| Lease expiry; renew/release/complete ownership and replays | `corvus-core/tests/huginn.rs` |
| Dependency blocking; cancellation, cascade, cancel-versus-claim race | `huginn.rs`, `races.rs` (threads and processes) |
| Idempotency replays for every create | `huginn.rs`, `sessions.rs`, `muninn.rs`, `races.rs` |
| Presence; auto-end versus concurrent heartbeat | `corvus-domain` presence tests, `sessions.rs`, `races.rs` |
| Unread / ack / unresolved questions; direct-thread pairing | `muninn.rs` |
| wait-for-inbox wake-ups (same and cross process), timeouts, lost hints | `corvus-core/tests/hermod.rs`, `corvus-cli/tests/messaging.rs`, `mcp_binary.rs`, `corvus-http/tests/longpoll.rs` |
| Export contents and path safety | `corvus-core/tests/export.rs` |
| Auth on every HTTP route; constant-time token; host allow-list; IPC seam; app opens no socket | `corvus-http/tests/security.rs`, `serve.rs`, `corvus-desktop/tests/dispatch.rs`, `no_listener.rs`, `make lint` seam check |
| Housekeeping isolation; background leader hand-over | `corvus-core/tests/background.rs` |
| Dashboard bundle has no data and no sinks; hostile text renders literally | `ui/scripts/scan-bundle.mjs`, `ui/e2e/dashboard.spec.ts`, `sources.spec.ts`, `panels.test.ts` |
| Hook dedupe / throttle / stop-loop protection / never fails | `corvus-cli/tests/hook.rs` |
| Wake supervisor retry, backoff, dedupe, restart; adapter rules; built-in `codex` adapter | `corvus-core/tests/watch.rs`, `adapters.rs`, `corvus-cli/tests/wake_cli.rs` |
| Source engine (assignment, re-poll, comment routing, follow-ups, ignored accounts, re-assignment, scoping, manual queueing, partial-page abort, single flight) | `corvus-core/tests/sources.rs` (fake source), `corvus-jira/tests/end_to_end.rs`, `corvus-linear/tests/end_to_end.rs` |
| Linear adapter against a fake Linear | `corvus-linear/tests/linear.rs` |
| Jira adapter against a fake Jira; JQL escaping; rich-text flattening | `corvus-jira/tests/jira.rs`, `corvus-domain` jql/adf property tests |
| The binary runs on a clean machine | static Linux arm64 binary in a stock `debian:stable-slim` container (plan 0011) |

### Measured

- Write-transaction hold time under 16-way contention: ~1–2 ms (ceiling 250 ms); longest lock wait ~1.3 s.
- `corvus hook` (release): read path median 3.8 ms, p95 4.8 ms; throttle path median 2.7 ms.
- Cross-process wake latency in tests: ~0.3 s.
- Release binary: 10.4 MiB (macOS arm64; links only `libSystem`, `libiconv`), 10.6 MiB (Linux arm64, static).
- Release archives (`make dist`, 2026-11-16): 4.8–5.4 MiB per `corvus` tarball, 10.0 MiB zipped app; macOS
  artifacts signed (`Authority=Matthew Krueger`, hardened runtime) and verified after extraction; both Linux
  archives pass the clean-image smoke test.

## Verified by hand on this machine

- `corvus serve` with `curl` across every route group; refuses missing/short tokens; SIGTERM exits 0; token never logged.
- Two `corvus serve` processes: leader hand-over after `kill -9` (23 s, within the 30 s retry) and a 10 s lease
  reaped by the new leader.
- `corvus up` / `status` / `down` with a stub `codex`: woken with exactly the adapter's arguments and the fixed note (since plan 0012: `queue --thread <id> --resume --message <fixed note>`).
- The Claude Code hook through the real binary against the documented contract (not through Claude Code itself).
- `cargo install` from a fresh clone with Node absent; Linux clean-container smoke test; the `Corvus.app` bundle.
- A no-context agent followed the README end to end in an isolated home directory; its findings were fixed
  (notably Quick start step 3, which needed the agent to exist before `corvus tell`).
- **Owner, on screen (2026-11-14):** the desktop app's manual checklist — live data, `corvus tell` appears,
  hostile text literal, monochrome menu bar icon, close-to-tray with the Dock icon hidden, still leader, Open,
  Pause/Resume, relaunch focuses, Quit leaves the CLI working. All rows passed ([plan 0006](../spec/plans/0006-dashboard-and-desktop.md#unverified--deferred)).
- **A real Claude Code session (2.1.325) through the hook (2026-11-14):** a prompt-time message reached the
  model; a mid-turn message produced exactly one stop-block; no receipts were created ([plan 0007](../spec/plans/0007-delivery-helpers.md#unverified--deferred)).
- **Upgrade across two real binaries (2026-11-14):** schema-6 binary creates data, a schema-7 build backs up
  (`corvus.db.pre-v7.bak`) and migrates, data survives, the still-running old `corvus mcp` answers its next write
  with `upgraded` / "restart this process", and the old binary refuses the newer database. This found a bug —
  the restart message was hidden behind `internal error` — fixed with a dedicated `upgraded` error.

## Not verified

| What | Why | Where it is tracked |
|---|---|---|
| Clicking a ticket link opens the system browser | needs a Linear or Jira task; the link guard is unit- and e2e-tested | plan 0006 |
| Linux desktop (WebKitGTK), `tauri-driver` window tests | no Linux desktop machine; deferred past v1 | plan 0006 |
| The Codex and OpenCode wake paths with the desktop app (not `corvus serve`) as leader; Codex woken with no window open | confirmed with `corvus serve` and open windows only (2026-11-16) | plan 0012 |
| Real Linear workspace and real Jira site | no accounts; adapters tested against fakes and the published schema / OpenAPI | plans 0008, 0009 |
| The Gatekeeper first-launch override on a downloaded (quarantined) artifact | signatures verify, but the override needs a real download and a person at the screen | plan 0011 |
| Clean macOS user account (binary and app; install CLI; agent connects with the shown snippet) | needs another account / machine | plan 0011 |

## Not built

- Nothing in scope was left unbuilt. Excluded by owner direction: Windows support, a browser dashboard / `/ui`
  route, writing back to trackers, adapters for tools other than Codex (the pattern is built; new tools are one
  file each). Future: notarised distribution, auto-update, package-manager formulas.

## Known gaps and open owner decisions

- Known gaps: [docs/architecture.md](architecture.md#known-gaps) (from [system/01](../spec/system/01-architecture.md#known-gaps-kept-current-feeds-the-final-architecture-note)).
- **Owner decisions taken (2026-11-14):** keep the single-instance local socket; the rule is "no network
  listener" ([D0022](../spec/decisions/0022-desktop-single-instance-local-socket.md)). Releases go to GitHub
  Releases only; signing uses the owner's self-signed certificate; Codex is deferred past v1.
- Open plans: 0011 stays *In progress* until the owner's clean-account check and the GitHub Release is published.
- One full-suite run on 2026-11-14 had a single test failure that six later full runs did not reproduce; its
  name was not captured. Treat it as a possible flake to watch.
