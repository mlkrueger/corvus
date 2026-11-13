# Corvus — product brief (requirements source)

> **Status: frozen.** This is the original brief, verbatim. It states WHAT Corvus does. It is never edited;
> if a requirement changes, record a decision in [`decisions/`](decisions/README.md) and update the
> [system spec](system/README.md), which states HOW.
>
> **Superseded by owner direction:** the brief lets the builder choose the language and frameworks. The owner
> has fixed these: **Rust**, with **Tauri 2** for the UI. See [D0014](decisions/0014-rust-stack-rusqlite.md) and
> [D0020](decisions/0020-desktop-only-dashboard.md). The dashboard is the desktop app only — no browser UI.
>
> **Fixed by owner direction:** Corvus ships as **one self-contained Rust binary — no containers, no database
> server** — and the desktop app is self-contained. Where that changes the brief's wording, see
> [system/11](system/11-defaults.md#deliberate-departures-from-the-briefs-wording).
>
> **Added by owner direction:** a **Linear** work source ships in v1 alongside Jira, with the same behaviour as
> §4.9. See [D0011](decisions/0011-work-source-abstraction.md) and [system/07b](system/07b-linear.md).
>
> Cite this file as `brief §N` (for example `brief §4.2`). The ten properties in §2 are cited as `P1`..`P10`.

---

# System prompt: build "Corvus"

You are a senior engineer building a complete, production-quality system from this specification. The specification
describes WHAT the system does and the properties it must have. It deliberately says nothing about HOW to build it.
You choose the language, frameworks, storage engine and layout, and you must justify those choices in a short
`DECISIONS.md` at the end. Do not ask questions; where the spec is silent, choose a sensible default and record it
in `DECISIONS.md`. Deliver working software, an automated test suite you have actually run, and a README that lets a
stranger start it in a few commands. Do not stop at a skeleton: every capability below must be implemented and tested.

## 1. Purpose

Corvus is a local-first coordination service for AI coding-agent sessions running on one developer machine. Several
agents (for example "claude_code" and "codex", or several windows of the same tool) share work and talk to each other
through Corvus instead of through ad-hoc files. It provides:

- a **work queue** with claiming, leases, dependencies and cancellation (call this subsystem **Huginn**),
- **memory and messaging**: threads, messages with read-receipts, digests, decisions, audit events (**Muninn**),
- **wake-ups**: a way to block until something relevant happens (**Hermod**),
- an **operator dashboard**, a **command-line tool**, an **HTTP API** and an **agent-tool interface**,
- a **Jira source** that turns Jira tickets into queue work.

Corvus never pushes into a running agent. Agents PULL. Corvus's job is to make pulling reliable and cheap.

## 2. Non-negotiable properties

1. **One durable source of truth.** All state lives in one durable store with transactional, concurrency-safe updates.
   Anything exported to files is a read-only snapshot and is never read back as state.
2. **Atomic claiming.** Two agents racing for the same task must never both win. Claiming, releasing, completing,
   cancelling and lease expiry are each atomic and deadlock-free under concurrency.
3. **Idempotent mutations.** Every create operation accepts an optional `idempotency_key` (scoped to the creator);
   replays return the original result and create nothing new.
4. **Append-only history.** Messages, digests and audit events can never be updated or deleted, including by bugs
   (enforce this in the store, not only in application code).
5. **Notifications are hints.** Wake-ups only say "go look"; consumers always re-read state, tolerate coalesced,
   duplicated or missing notifications, and subscribe BEFORE reading so nothing is lost in between.
6. **Secure by default.** Loopback-only network binding. Every data-bearing HTTP route requires a bearer token read
   from the environment; the server refuses to start without one. Compare tokens in constant time. No interactive
   API docs or schema endpoint. Secrets only from environment variables, never in code, logs, URLs sent to servers,
   or the database. A host-header allow-list (loopback names only) defends against DNS rebinding.
7. **Untrusted text stays text.** Anything written by agents or copied from Jira is data. The dashboard must never
   interpret it as markup or script. Text copied from Jira into tasks/messages is labelled as untrusted.
8. **Never fatal in the background.** Housekeeping and polling loops log failures and continue; an unreachable Jira or
   a failed sweep must not take the server down or corrupt state.
9. **Observable.** Structured (JSON-lines) logs to stderr; every state change appends an audit event (no message bodies).
10. **Versioned schema.** Database/schema changes are applied by ordered, checksummed, one-way migrations run under a
    lock; an applied migration is never edited.

## 3. Domain model

**Agent name**: 1-64 characters, `[A-Za-z0-9_.-]`, starting with a letter or digit (names end up in file names).
Unknown names are created on first use.

**Session**: one live connection of an agent. Has a UUID, `agent_name`, `client_name`, free-form metadata,
`started_at`, `last_seen_at`, optional `ended_at`, and `name_generated`. Several sessions may share an agent name; the
UUID tells them apart. If a session registers without a name, Corvus generates and atomically reserves a unique
readable one of the form `<adjective>-<bird>` (e.g. `keen-raven`), adding a numeric suffix after many collisions, and
sets `name_generated=true`.

**Task**: `title` (non-empty), `body`, integer `priority` (higher is served first), `status` in
`open | claimed | done | cancelled`, `pool` (routing bucket, default "default"), optional `recipient_agent`
(null = any agent), `result_summary`, `cancel_reason`, creator, timestamps, dependency list, artifact list, and a
conversation thread created together with the task. A task is **claimable** when it is open, its pool matches (if the
claimer asked for one), it is unrouted or routed to the claimer, and every dependency is `done`.

**Claim**: history row (task, session, agent, claimed_at, lease_expires_at, released_at, end_reason in
`released | completed | expired | cancelled`). At most ONE active claim per task, and a session holds at most one
active claim at a time.

**Thread**: title, optional task link, `open|closed`, creator, optional unique `key` (for idempotent standalone threads).

**Message**: thread, sender, optional recipient (null = broadcast to everyone), type in
`note | question | answer | handoff | status | decision`, optional `in_reply_to` (must be in the same thread), body
(non-empty, max 20,000 chars), optional idempotency key, creation time, optional artifacts, and a monotonically
increasing id used for ordering.

**Receipt**: per (message, agent) acknowledgement. Unread is a query, not a cursor: a message is unread for agent A when
it is addressed to A or broadcast, was not sent by A, and A has no receipt for it. A question is *unresolved* until some
`answer` message replies to it.

**Artifact**: a REFERENCE into Git, never content: kind in `file | branch | worktree | commit | pr`, repo-relative path
(reject absolute paths, `..` segments, NUL), optional commit hash (7-40 lowercase hex), description. Attachable to a
task, thread or message.

**Digest**: agent-authored summary, scope `global | task | thread | agent` (with the matching reference), optional
"covers through message id". Append-only; the latest per scope key is current.

**Decision**: lightweight ADR: title, decision, rationale, decided_by, optional task/thread, optional `supersedes`
(superseded decisions are hidden from default listings).

**Event**: append-only audit row (kind, summary, agent, session, task, thread, message, time) for every state change.

## 4. Behaviour

### 4.1 Sessions and presence
- `register_session(agent_name?, client_name, metadata)` creates a session and returns it. `connect(...)` does the same and
  also returns a *bootstrap context* and operating instructions (see 4.6) in one call; this is what agents use first.
- `heartbeat(session_id)` and every other call that carries a session id bumps `last_seen_at`.
- `end_session` releases everything the session holds (tasks reopen) and the session can no longer act. Acting with an
  ended or unknown session returns a clear error telling the agent to register again.
- **Presence is derived, never self-reported**: `ended` if ended; else `offline` if silent longer than the idle window
  (default 900 s); else `working` if it holds an active claim; else `active` if seen within the active window
  (default 120 s); else `idle`.
- **Auto-end**: sessions silent longer than a configurable threshold (default 3600 s; 0 disables) are ended by the server,
  atomically (a concurrent heartbeat wins), releasing their claims, with an audit event. Also available on demand.

### 4.2 Tasks, claiming, leases
- `create_task(creator, title, body, priority, recipient|pool, depends_on[], artifacts[], idempotency_key)` also creates the
  task's thread and an audit event. Unknown dependency or agent -> not-found error.
- `claim_task(session_id, pool?)` returns the best claimable task (highest priority, then oldest) with its claim, or
  nothing. Idempotent per session: if the session already holds an active claim, return that claim. It first reaps
  expired leases. Concurrent claimers must never block each other nor receive the same task.
- Lease: default 900 s (configurable). `renew_lease(task, session)` extends it. Only the claiming session may renew,
  release or complete; anyone else gets a conflict. Replays of release/complete by the same session are no-ops.
- `release_task` reopens the task. `complete_task(task, session, result_summary)` marks it `done` (final).
- `expire_leases` ends expired claims (reason `expired`) and reopens their tasks; it also runs automatically as part of
  claiming and from the background sweep (every ~60 s while the server runs).
- `cancel_task(task, by, reason, cascade)`: works on open/claimed tasks; done tasks cannot be cancelled; idempotent.
  An active claimant is sent a `status` message in the task thread and gets a conflict on its next renew/complete.
  Dependents of a cancelled task stay BLOCKED (never silently unblocked) and the dashboard flags them; with `cascade`
  the whole dependent tree is cancelled. Cancellation must be safe against a claim appearing concurrently.
- Lock ordering must be consistent (session -> claims -> tasks) so concurrent operations cannot deadlock.
- `list_tasks(status?, agent?, limit)`, `get_task(id)` return tasks with thread id, dependencies and artifacts.
- Tasks can additionally be appended to while still open (used by the Jira source, 4.9) and the claiming agent of a task
  can be looked up.

### 4.3 Messaging
- `create_thread(creator, title, task?, key?)`; with a key it is idempotent (same key, same thread).
- `post_message(thread, from, body, type, to?, in_reply_to?, artifacts?, idempotency_key?, session_id?)`.
- `send_direct(from, to, body, type)`: one persistent keyed thread per unordered agent pair, so conversations stay together.
- `read_inbox(agent, limit, after_id?)` returns unread messages oldest first. `after_id` is only a paging hint.
- `ack_message(message, agent, session?)`: idempotent; returns whether newly acked; an agent cannot ack its own message
  or one not addressed to it (conflict).
- `list_messages(limit, before_id?, agent?, thread?, unread_only?)` for the operator: newest first, each with its thread
  and the list of agents who have and have not read it. Viewing NEVER acks.
- `wait_for_inbox(agent, timeout<=120 s)`: subscribe, check, then block for a wake-up or timeout, re-check; returns as
  soon as unread messages exist. Waiting counts as being alive (heartbeat).
- A hint is emitted when any message is added (carrying the recipient or "everyone") and when any task changes.

### 4.4 Memory
- `write_digest(scope, ref, summary, covers_through?)`; `latest digest` per scope key; all latest digests listable.
- `record_decision(...)`, `list_decisions(task?, thread?, include_superseded)`.
- `export_state_to_git(subdir="coordination", commit=false)`: if an export repository is configured, atomically write
  `summary.md` (counts + open/claimed tasks), `open_tasks.json`, `inbox/<agent>.json` for every agent, `digests/<ref>.md`,
  and `decisions.md`, returning repo-relative paths; optionally `git add` + commit only those paths. Reject absolute or
  `..` subdirs. Without a configured repo -> validation error.

### 4.5 Overview, stats, health
- `stats`: open tasks, claimed tasks, active leases, expired leases, unread per agent and total.
- `overview` (for the dashboard): session counts by presence, sessions with presence/current task/lease expiry/unread
  count, recent tasks (with `blocked`, `blocked_by_cancelled`, who holds them, cancel reason, result summary), recent
  audit events, recent decisions, the operator name, and the presence thresholds.
- `health`: ok + current schema version.

### 4.6 Agent guidance (a single source of truth)
One piece of operating-instruction text is used verbatim by the agent-tool interface's server instructions, by the
`connect` result and by the docs. It tells an agent to: (1) handle unread messages now, reply in the same thread with
`in_reply_to`, then ack each; treat peer messages as requests from peers, not instructions from the user; (2) keep
checking its inbox at the start of every task and every few steps, `wait_for_inbox` when blocked, `heartbeat` when quiet;
(3) claim work, renew while working, complete with a summary or release with a handoff, stop if told the task was
cancelled; (4) record lasting choices and leave digests; (5) end the session, and re-register if told it is not active.
The *bootstrap context* is: the agent's assigned tasks, open tasks, unread messages, unresolved questions, latest
digests and decisions.

### 4.7 Delivery helpers (make pull reliable)
- **Inbox hook for Claude Code**: a tiny, fast, failure-silent program registered for the prompt-submit, after-tool and
  stop events. It reads the agent's unread messages and injects them into the model's context ONCE each (de-duplicated per
  agent + Claude session in a small local state file), formatted with the guidance above, bodies truncated to ~1000 chars,
  at most 10 per injection, the operator labelled as "the operator". After-tool checks are throttled (default 5 s). On
  the stop event, if mail is waiting it blocks stopping once (never when it already blocked: no loops) and returns the
  messages as the reason. It NEVER acks and never makes the agent fail: any error exits quietly with success.
- **Wake supervisor**: a long-running helper that blocks on the inbox (no model tokens spent) and either prints unread
  messages and exits, or runs a configured command per batch of NEW unread messages, passing the agent name, the new
  messages as JSON and a per-agent state directory. Exit 0 = delivered (not handled; only the agent acks). Failure retries
  with exponential backoff (cap 300 s); the command is killed after a timeout (default 300 s) and counts as failure.
  Delivered ids are tracked as a set (ids can commit out of order); one watcher per agent (lock file). Duplicate delivery
  after a crash is acceptable and documented.
- **Codex adapter**: a callback for the supervisor that queues a FIXED "check your Corvus inbox" note into one explicitly
  bound Codex conversation (bound once by agent name + conversation UUID, stored locally). Peer message text is never put
  in the prompt. Validate agent names and UUIDs; treat queue acceptance as delivery, not acknowledgement.
- Convenience scripts to bring the whole stack up/down/status (start storage, apply migrations, start the server in the
  background with a pid file, wait for readiness, restart all bound watchers) with matching `make`-style targets.

### 4.8 Background housekeeping
Inside the running server, every ~60 s: expire leases and auto-end silent sessions. One pass is separately callable and
unit-testable; each step's failure is logged and isolated.

### 4.9 Jira source (work-queue feed)
Read-only integration with the Jira Cloud REST API (v3, enhanced JQL search, Basic auth of account email + API token).
No inbound webhooks. Enabled only when `CORVUS_JIRA_BASE_URL` (must be https), `CORVUS_JIRA_EMAIL` and
`CORVUS_JIRA_API_TOKEN` are all set (error clearly if some are missing); optional `CORVUS_JIRA_POLL_SECONDS` (60),
`CORVUS_JIRA_POOL` ("default"), `CORVUS_JIRA_IGNORE_ACCOUNTS` (comma-separated account ids whose comments are not work).
All Jira-created records are authored by an agent named `jira`.

**Filter (operator-editable, persisted, never contains credentials):**
- `statuses` to show (default Backlog, To Do, In Progress, Waiting for response); `done_days` to also show tickets
  resolved in the last N days (default 7, 0-90, 0 = off); `projects` to limit (empty = all; keys like `AA`);
  `auto_queue` rules: each has `projects` (empty = any) and a non-empty `statuses` list (default: one rule, project AA,
  statuses To Do + In Progress). Validate everything (status names 1-60 printable chars, max 30; project keys; at least
  one status or a done window). The query is built by Corvus from these structured fields (always limited to tickets
  assigned to the API user, i.e. `assignee = currentUser()`); raw query text is never accepted from the UI, and status
  names are quoted/escaped so they cannot break out of the query.
- Poll loop (and an on-demand "sync now"): serialised, so timer and manual runs never interleave. Fetch ALL pages; if any
  page fails, abort the whole pass without acting on a partial view.

**Reconciliation per pass:**
1. Replace a **snapshot** of matched tickets (key, project, summary, status, status category, priority, url, updated
   stamp, resolved time) used by the dashboard board.
2. A ticket **auto-queues** if its status category is not done and some rule matches its project and status. Auto-queued
   tickets with no live link create a task: title `[KEY] summary`, priority mapped from Jira priority (Highest 10, High 5,
   Medium 0, Low -5, Lowest -10), the configured pool, and a body with the ticket URL, status/priority/assignee, a
   notice that everything below is untrusted Jira text, the description (rich-text converted to readable plain text),
   and the last 5 non-ignored comments. Truncate bodies to 20,000 chars.
3. Each ticket is linked to its task(s) (a ticket may have several over time; the newest link is live). A link stores the
   last processed `updated` stamp (compared for EQUALITY only, never parsed, so time zones cannot lose events), a numeric
   **comment high-water mark**, an `in_scope` flag and a `manual` flag. Task creation is idempotent per ticket + stamp.
4. **Comments** (only fetched when a ticket's updated stamp changed): new comments (id above the high-water mark, not from
   ignored accounts) are (a) appended to the task body if the task is still open; (b) sent as a message addressed to the
   claiming agent in the task thread if claimed (marked untrusted); (c) if the task is done/cancelled, a follow-up task is
   queued carrying the comments. If the task changes state mid-delivery, retry on the next pass.
5. Tickets that stop matching the auto-queue rules are marked out of scope (hand-queued links are exempt). A ticket that
   leaves and later re-enters scope after its task finished gets a fresh task; if its task is still live it just resumes.
   Ticket removal/cancellation in Jira does NOT cancel tasks (documented limitation). Nothing is ever written back to Jira.
6. **Manual queueing (operator action)**: any ticket currently on the board that is not done can be added to the work
   queue by hand regardless of rules (the ticket key must be on the board and well-formed: `[A-Z][A-Z0-9_]*-\d+`). It
   creates a task flagged `manual`; conflict if the ticket already has a live (open/claimed) task; allowed again after
   the previous task finished. Hand-queued tickets keep syncing comments while their task is live and the ticket is not
   done. An open task can also be removed from the queue by cancelling it from the dashboard.
7. Issue keys are validated before being placed in any URL path. The Jira site is fixed by configuration (no
   caller-supplied hosts). Redirects are not followed. Requests have timeouts.

HTTP surface for Jira: read board state (enabled flag, site, filter, tickets with `auto_queue`, `manual`, task status/id,
statuses known on the site for suggestions (cache ~10 min), last sync time/stats/error); save filter; sync now; queue a
ticket by key. When Jira is not configured these return a clear conflict error and the read route reports `enabled:false`.

## 5. Interfaces

All four wrap the same service layer; no business logic in transports.

**HTTP API** (JSON, bearer token on everything except the static dashboard shell): sessions (create, heartbeat, end,
list), connect, tasks (create, list, get, claim, renew, release, complete, cancel), threads, messages (post, list,
direct, per-thread), inbox (read, wait, ack), digests, bootstrap, decisions, events, stats, health, overview, admin
(expire leases, end stale sessions, export), and the Jira routes above. Domain errors map to 404 / 409 / 422; request
validation errors to 422. Mutation routes accept idempotency keys.

**Agent-tool interface (Model Context Protocol server over stdio)** exposing: register_session (returns the full
connect result), get_bootstrap_context, create_task, claim_task, renew_lease, release_task, complete_task, cancel_task,
post_message, create_thread, send_direct_message, read_inbox, wait_for_inbox, ack_message, heartbeat, end_session,
write_digest, record_decision, list_decisions, export_state_to_git. Server instructions = the guidance text (4.6).
A launcher script loads secrets from a local env file so no secret is in any client configuration.

**CLI**: session (register, heartbeat, end, list), connect, task (create, claim, renew, release, complete, cancel),
msg post, inbox (read, ack, wait), tell `<agent> <message>` (operator direct message; sender defaults to
`CORVUS_OPERATOR_NAME`, default "operator"), digest write, decision (record, list), bootstrap, stats, health, migrate,
serve, mcp, ui (opens the dashboard signed in), jira-sync, admin (expire-leases, end-stale, export). JSON output.

**Dashboard** (single static page served at `/ui`; the shell contains no data; everything is fetched from authenticated
routes and refreshed about every 3 s, with pause, theme toggle and sign-out):
- Auth: token supplied via URL fragment (never sent to the server; stripped from the address bar) or pasted into a gate
  form; kept only for that tab's session.
- Header chips: sessions by presence, open/claimed tasks, unread total (click to filter), expired leases, blocked-by-cancelled.
- Agents & sessions with presence, client, last seen, unread count, current task + lease time; toggle for offline/ended.
- Activity feed (audit events). Decisions list.
- **Messages panel**: all messages newest-first with thread, read/unread-by, filters (all / unread / by agent),
  a compose box sending as the operator to one agent or everyone, and Reply (posts into the thread with `in_reply_to`,
  defaulting to `answer` for a question). Viewing never acks.
- **Tasks board**: Open / Claimed / Done (recent) / Cancelled (recent) columns with priority, blocked flags, holder.
- **Jira panel** (shown only when Jira is enabled): one column per shown status plus "Done in last N days"; cards show key
  (link to Jira, https only, `noopener`), summary, priority, an "auto" badge when a rule covers it, and the task state
  ("by hand" when manually queued); **Add to queue** / **Queue now** and **Remove from queue** (cancel while unclaimed)
  buttons; **Sync now**; a **Filter** form editing statuses, done window, projects and auto-queue rules one per line as
  `AA, LAI: To Do, In Progress` (`*` = any project), with clickable suggestions of the site's statuses; save triggers a sync;
  show last-sync time or error. Polls Jira state about every 30 s.
- Security: strict content-security policy, no-store caching, no inline rendering of untrusted text as markup (build the
  page with text nodes only), loopback host allow-list. Works in light and dark themes and on narrow screens.

## 6. Configuration (environment variables)

`CORVUS_DATABASE_URL` (required), `CORVUS_API_TOKEN` (required to serve), `CORVUS_LEASE_SECONDS` (900),
`CORVUS_ACTIVE_SECONDS` (120), `CORVUS_IDLE_SECONDS` (900), `CORVUS_AUTO_END_SECONDS` (3600, 0 = off),
`CORVUS_OPERATOR_NAME` ("operator"), `CORVUS_EXPORT_REPO` (optional), `CORVUS_STATE_DIR` (hook state, default under the
user's home), `CORVUS_HTTP_PORT` (default 8765), the Jira variables in 4.9. Provide an `.env.example` with no real values.
Defaults must be safe: loopback bind, private everything.

## 7. Quality bar

- **Tests you have run**, against the real storage engine (not mocks) wherever concurrency or atomicity matters:
  concurrent claim races; lease expiry; dependency blocking; cancellation incl. cascade and the claim-appears-mid-cancel race;
  idempotency replays; append-only enforcement; unread/ack/unresolved-question semantics; wait-for-inbox wake-ups and
  timeouts; presence derivation; auto-end vs concurrent heartbeat; direct-thread pairing; export contents and path
  safety; auth required on every data route and token compared safely; the dashboard shell contains no data and no
  untrusted-markup sinks; hook de-duplication / throttling / stop-loop protection / never-fails; wake supervisor retry,
  backoff, dedupe and restart behaviour; the Jira source against a fake Jira (assignment, idempotent re-poll, comment
  routing by task state, follow-ups, ignored accounts, re-assignment, rule scoping by project, manual queueing and its
  comment sync, filter validation and query escaping, rich-text flattening, config validation); migrations are ordered,
  checksummed and re-runnable. Aim for broad, meaningful coverage rather than a count.
- Layered code: pure domain model; services that own transactions; thin transports. No business logic in handlers.
- A README (quick start, how to connect an agent, how to run tests), concise API/usage docs, an architecture note
  stating decisions and known gaps, and `DECISIONS.md`.
- Everything runs on a laptop with one command to start dependencies and one to run the tests.

## 8. Deliverable and process

Work in this order and keep it working at every step: schema + migrations -> domain + services (tasks/claims first, with
their concurrency tests) -> messaging/memory -> HTTP/CLI/agent-tool transports -> housekeeping -> dashboard -> delivery
helpers -> Jira source. Run the full test suite before you finish and report honestly what passes, what you did not
build, and what you could not verify. Do not claim behaviour you have not exercised.
