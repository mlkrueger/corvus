# 11 — Defaults chosen where the brief is silent

Small choices that do not merit a full [decision record](../decisions/README.md). Add a row when you make one;
promote it to a decision if anyone would reasonably argue about it. This table and the decision index together
become the root `DECISIONS.md` the brief asks for ([plan 0010](../plans/0010-docs-and-release.md)).

## Defaults

| # | Topic | Default | Why |
|---|---|---|---|
| 1 | Enumerations in the schema | `TEXT` + `CHECK` | Adding a value is a one-line migration |
| 2 | State directory | `~/.corvus` (`0700`), database `corvus.db` inside it (`0600`) | One obvious place; private by default |
| 3 | Timestamps in the store | integer UTC milliseconds | Exact comparison; no text parsing |
| 4 | SQLite durability | `synchronous = FULL` | "Durable" store; write volume is tiny, so the cost is irrelevant |
| 5 | Write-queue timeout | `busy_timeout` 5 s, two retries, then `Busy` | Far above any real queueing; bounded |
| 6 | Auto-migration | on open by any writing process; backup file first | Zero-setup; explicit `corvus migrate` still exists |
| 7 | Task runner | `Makefile` (development only) | Preinstalled on macOS and Linux |
| 8 | HTTP route prefix | `/api/…`, unversioned | Single-machine tool; clients ship in the same binary |
| 9 | `health` route | requires the token | Every HTTP route does; none is exempt |
| 10 | Token minimum length | 32 characters | Rejects placeholder tokens; `.env.example` shows how to generate one |
| 11 | `wait_for_inbox` timeout above 120 s | clamped to 120, not rejected | Friendlier to agents; still bounded |
| 12 | Hint detection interval / re-check | 250 ms / 10 s | Latency bound / lost-hint bound |
| 13 | Hint retention | newest 1,000 rows | Plumbing, not history |
| 14 | Idempotency replay with a different payload | returns the original, no error | Agents retry with regenerated text; the key is the identity ([D0006](../decisions/0006-idempotency-keys.md)) |
| 15 | Renewing an expired but un-reaped lease | allowed | Whichever transaction runs first wins; no harm done |
| 16 | Broadcast audience for read/unread-by lists | every agent that has ever had a session, minus the sender | "Everyone" needs a concrete set |
| 17 | Generated names | 20 random `<adjective>-<bird>` attempts, then numeric suffix | Brief: suffix "after many collisions" |
| 18 | Overview limits | 50 tasks per column, 100 events, 20 decisions | Keeps the 3 s poll cheap |
| 19 | Request body cap | 256 KiB | Far above a 20,000-char message; bounds abuse |
| 20 | Unknown / ended session | HTTP 409 with code `session_not_active` | A state conflict; the code lets clients re-register automatically |
| 21 | Rejected `Host` header | 403 | Checked before auth so rebinding attempts learn nothing |
| 22 | Hook's agent identity | `CORVUS_AGENT_NAME`; hook is silent without it | The hook cannot see the MCP process's session |
| 23 | Hook time budget | 50 ms typical; 300 ms busy timeout | "Tiny, fast"; never slow the agent |
| 24 | Export stale files | files under `inbox/` and `digests/` no longer produced are removed | Snapshot should match state |
| 25 | Minimum poll interval for sources | 15 s | Protects tracker rate limits from a typo |
| 26 | Linear default auto-queue rules | none | No safe guess at a team key; operator opts in |
| 27 | Linear done window | uses `completedAt` only; cancelled issues are not shown | "Done in last N days" means completed work |
| 28 | Jira/Linear unknown priority | maps to 0 | Neutral |
| 29 | MCP session convenience | the process remembers its session and fills in omitted `session_id` | Fewer agent mistakes; explicit ids still work |
| 30 | Root `DECISIONS.md` | generated index pointing into `spec/decisions/` + this table | One home for reasoning; the brief's deliverable still exists |
| 31 | Desktop window close | hides to tray; Quit exits | Background work should outlive the window |
| 32 | CLI install from the app | symlink into `~/.local/bin` | Never needs elevated rights |
| 33 | Platforms | macOS and Linux only; nothing Windows-related | Owner direction |
| 34 | macOS signing | release binary and app are signed with the owner's personal development certificate (testing only; no notarisation in v1); the identity is read from the keychain / environment at release time and never stored in the repo | Owner direction; secrets rule |
| 35 | Auditing heartbeats | Heartbeats (and the implicit session touch) write no event | Liveness, not a state change; one row per call would drown the activity feed |
| 36 | Idempotent registration without a name | The key is looked up among generated-name sessions | The agent name is not known before generation, so `(agent_name, key)` cannot match |
| 37 | Release / complete of an expired but un-reaped lease | Allowed, like renew (#15) | The claim is still active in the store; whichever transaction runs first wins |
| 38 | `end_session` on an ended / unknown session | Ended: no-op returning it. Unknown: `SessionNotActive` | Idempotent end; an unknown id is the same mistake as acting with one |
| 39 | Cancel's `status` message to the claimant | No separate `message.posted` event; the `task.cancelled` event carries the message id | One event per mutation; the message is part of the cancel |
| 40 | Who may cancel | Any agent (named in `by`); with a `session_id` it must belong to `by` | The brief sets no permission model; the operator and creators both need it |
| 41 | `wait_for_inbox` default timeout | 60 s when omitted (clamped to 0–120) | Long enough to be useful, well inside HTTP and MCP client timeouts |
| 42 | Default list sizes | inbox / messages 50, decisions 100, events 100; every limit capped at 500 | Bounded responses |
| 43 | `create_thread` with a task that has a thread | Returns that thread | One thread per task; asking again is not an error |
| 44 | `connect` transactions | Two: the registration, then one read for the bootstrap | Registration must commit before it can be read back |
| 45 | `list_messages(unread_only)` | Messages someone in their audience has not acked | The operator view has no single "me" |
| 46 | Overview sessions | Live sessions plus those ended in the last 24 h | Shows who just left without growing forever |
| 47 | HTTP success status | Always 200 (no 201) | Clients ship in the same binary; one rule is simpler |
| 48 | `GET /api/digests/latest` with no digest | 200 with `null` | Absence is a normal answer, not an error |
| 49 | `POST /api/admin/export` with an empty body | Treated as the default request | The common call has no options |
| 50 | Wrong method on a known path | 405 with an empty body (plus the security headers) | axum's default; not a data route |
| 51 | MCP session fill | Remembered `session_id` is filled only when the call's agent fields resolve to the remembered agent | A call made "as someone else" must not borrow this session |
| 52 | HTTP shutdown grace | In-flight requests get 5 s after the signal | Long polls must not hold shutdown for 120 s |
| 53 | Package manager for `ui/` | npm (with a committed `package-lock.json`) | Ships with Node; one less tool for contributors |
| 54 | Generated TypeScript integers | `number` (ts-rs `TS_RS_LARGE_INT`) | `JSON.parse` yields numbers; ids and counts stay below 2^53 |
| 55 | Linux `corvus` binary | Static musl (`<arch>-unknown-linux-musl`) | Runs on any distribution with no libraries installed; the desktop app is not affected |
| 56 | Where releases live | GitHub Releases: archives, the zipped app and `SHA256SUMS`; no crates.io publish | Owner direction (2026-11-14); one place to download, nothing to name-squat yet |
| 57 | Linux release builds | Official builds: natively on GitHub's Linux runner of each architecture with `musl-tools` ([D0026](../decisions/0026-official-builds-in-github-actions.md)). `make dist` on a Mac: cross-compiled from a container of the build machine's architecture (clang + `rust-lld` + Debian's musl headers) | Native is simplest where both architectures exist; on Apple silicon, running `rustc` under amd64 emulation crashed |
| 58 | macOS signing identity | The owner's self-signed personal certificate, chosen by name via `CORVUS_SIGN_IDENTITY`; for official builds it is held in the `release` environment's secrets ([D0026](../decisions/0026-official-builds-in-github-actions.md)) | Stable identity across updates; Gatekeeper still asks once on first launch; a Developer ID costs USD 99 a year |
| 59 | Version skew on write | Its own error, `upgraded` (HTTP 503, CLI exit 7), with a "restart this process" message shown by every transport | Under `internal` the transports hid the message, so an old process gave no hint why it failed |
| 60 | Registering a linked conversation under another name | The linked agent wins; the requested name is ignored and the result shows the agent | A conversation is one agent for good; failing the registration would strand the session |
| 61 | Waiter time limit | `"timeout": 86400` on the hook entry | Claude Code's default (600 s) kills the waiter and leaves the session asleep; any turn re-arms it |
| 62 | What wakes a session | Direct mail, and broadcasts the operator marks `wake`; other broadcasts wait for the session's next turn | A broadcast would cost one model turn in every open session, so waking everyone is a deliberate, operator-only act (owner, 2026-11-16) |
| 63 | Carry-over on `/clear` | The hook makes one database write, on `SessionStart source=clear` only | The only place both ids are known; every other hook event stays read-only |
| 64 | Who wakes OpenCode / Codex agents | The background leader (the desktop app or `corvus serve`); `corvus up` for headless machines | "Once the app is running I should not need to do anything else" (owner, 2026-11-16) |
| 65 | Sending to an unreachable agent | Delivered, with a warning; the dashboard flags it and the operator can reassign the mail | Mail is never refused and a mailbox never just looks dead ([D0025](../decisions/0025-operator-reassigns-stranded-mail.md)) |
| 66 | Setting up the Claude Code hooks | `corvus setup claude` (shows the change, asks, backs up, removable) and the same snippet in the docs | A command for convenience; a paste-it-yourself path for those who would rather not let a tool edit their settings (owner, 2026-11-16) |
| 67 | A Corvus tool call after `/clear` or a resume, before `register_session` | `corvus mcp` registers a session for the conversation's linked agent and runs the call | Claude Code gives the server no signal for a clear; the link already says who the conversation is (owner, 2026-11-19) |
| 68 | Launch at login (desktop app) | On, set once at the first launch; a login launch opens into the menu bar only; a checkbox in the Connect view turns it off for good | The background leader must be running after a reboot without the owner doing anything (owner, 2026-11-19) |

## Deliberate departures from the brief's wording

Each follows from owner direction (self-contained binary, no containers) and is justified in a decision.

| Brief says | Corvus does | Decision |
|---|---|---|
| `CORVUS_DATABASE_URL` is required | Optional; defaults to a file in the state directory | [D0013](../decisions/0013-storage-sqlite.md) |
| Concurrent claimers never block each other | Writers queue for one short transaction each; never the same task, never waiting on a lease | [D0017](../decisions/0017-single-writer-concurrency.md) |
| Consistent lock ordering session → claims → tasks | One database write lock; the order is kept as statement order only | [D0017](../decisions/0017-single-writer-concurrency.md) |
| Stack scripts "start storage", start the server | `corvus up / down / status` subcommands; storage is a file | [D0015](../decisions/0015-process-topology-single-binary.md) |
| A launcher script loads secrets for the MCP server | The binary loads the env file itself; MCP needs no secrets at all | [D0015](../decisions/0015-process-topology-single-binary.md) |
| Inbox hook is a "tiny program" | A subcommand of the one binary, reading the database directly | [D0015](../decisions/0015-process-topology-single-binary.md) |
| The server refuses to start without a token | True for `corvus serve`. The desktop app needs no token and never opens a network listener | [D0020](../decisions/0020-desktop-only-dashboard.md) |
| Dashboard is a static page served at `/ui`, with token-in-fragment sign-in, a paste-token gate and sign-out | Not built. The dashboard is the desktop app only, over IPC; there is nothing to sign in to | [D0020](../decisions/0020-desktop-only-dashboard.md) |
| Bearer token on everything except the static dashboard shell | Bearer token on every HTTP route; there is no shell route | [D0020](../decisions/0020-desktop-only-dashboard.md) |
| `corvus ui` opens the dashboard signed in | `corvus ui` launches the desktop app | [D0020](../decisions/0020-desktop-only-dashboard.md) |
| A Codex adapter | A general adapter pattern for agent tools; Codex is the built-in one and follows the brief's rules | [D0021](../decisions/0021-agent-adapters.md) |
| Jira source only | Jira and Linear on one engine | [D0011](../decisions/0011-work-source-abstraction.md) |
