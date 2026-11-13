# Plan 0007 — Delivery helpers: hook, wake supervisor, agent adapters, stack commands

- **Status:** Ready
- **Depends on:** [0004](0004-transports.md)
- **Spec:** [system/06](../system/06-delivery-helpers.md) · brief §4.7

## Goal

Pulling becomes reliable in practice: Claude Code is shown new mail by a hook, idle agents are woken by a
supervisor through a per-tool adapter (Codex built in, others addable without a rebuild), and one command brings the optional background stack up or down — all as
subcommands of the one binary, none needing a server.

## Scope

**In:** `corvus hook`; `corvus watch`; the adapter runner, `corvus agent bind|unbind|list`,
`corvus adapter list|show|check`, the built-in `codex` adapter; `corvus up|down|status`; Makefile
targets; setup docs for registering the hook and binding watchers.

**Out:** anything that acks on the agent's behalf (never); adapters for other tools such as OpenCode or Pi
(each a small future plan — the pattern is built here, the tools are not).

## Work breakdown

- [ ] 1. Confirm the Claude Code hook contract (event names, stdin fields, stdout JSON for context injection and
      for blocking stop) against current docs; adjust [system/06](../system/06-delivery-helpers.md) if needed.
- [ ] 2. `corvus hook`: fast path (no runtime, throttle before opening the database), read-only open, state
      file, formatting, three event behaviours, catch-all silent exit.
- [ ] 3. `corvus watch`: print mode; exec mode with env/stdin contract, timeout + process-group kill, backoff,
      delivered-set persistence, lock file.
- [ ] 4. Adapter runner: TOML loading and validation (closed placeholder set, whole-argument rule, ownership
      and permission check), embedded built-ins with user override, bindings file, `corvus agent …` and
      `corvus adapter …` commands, adapter mode in `corvus watch`.
- [ ] 5. Built-in `codex` adapter: confirm the resume command on the installed Codex version (see open
      questions), write the adapter file, test against a stub `codex`.
- [ ] 6. `corvus up` / `down` / `status` (idempotent), detached background `serve`, pid and log files, watcher
      start/stop; Makefile targets.
- [ ] 7. Docs: hook registration snippet for Claude Code settings; binding walkthrough; "writing an adapter
      for a new tool" page with a worked example.

## Verification

Hook (the real binary via `assert_cmd`, temp database, temp state dir):
- [ ] injects each message once per (agent, Claude session); a second session gets it again; creates no
      receipts and writes nothing to the database (file checksum of tables unchanged; connection is read-only).
- [ ] ≤ 10 messages per injection with a "more waiting" note; bodies truncated to ~1000 chars; operator shown
      as "the operator"; guidance text present.
- [ ] after-tool throttle: second call within 5 s does not open the database; after 5 s it does.
- [ ] stop: blocks once with the messages as reason; with `stop_hook_active` true it never blocks.
- [ ] **never fails:** no database, schema newer or older than the binary, database held by a long writer past
      the busy timeout, corrupt state file, unwritable state dir, missing agent name, malformed stdin →
      exit 0, empty stdout, in every case.
- [ ] **works with no server and no token** in the environment.
- [ ] **fast:** p95 wall time under the stated budget on the throttle path and the read path (measured in CI
      with generous slack; regression guard, not a benchmark).
- [ ] hostile agent name / session id cannot escape the state directory.

Wake supervisor (temp database, stub commands, paused time where possible):
- [ ] print mode prints and exits when mail exists; blocks otherwise; wakes when another process posts.
- [ ] exec mode passes agent, state dir and messages JSON; exit 0 marks delivered; no ack happens.
- [ ] failure → retries the same batch with 1, 2, 4 … s backoff capped at 300 s; success resets it.
- [ ] timeout kills the command (and its children) and counts as failure.
- [ ] dedupe by id set: an older id that appears after a newer one was delivered is still delivered.
- [ ] restart: delivered set survives; messages that arrived while down are delivered; a simulated crash before
      persisting re-delivers (documented duplicate).
- [ ] second watcher for the same agent exits with a clear error; different agents coexist.

Adapters:
- [ ] an adapter with an unknown placeholder, a placeholder embedded inside an argument, a missing command, or
      bad permissions/ownership is rejected at load with a clear message.
- [ ] a user adapter overrides a built-in of the same name; `adapter list/show/check` report correctly;
      `check` distinguishes "tool not installed".
- [ ] bind validates the agent name and the adapter's binding pattern; one binding per agent; `--force` re-binds.
- [ ] **no message text can reach a tool:** with messages whose bodies contain instructions, the stub tool's
      arguments, stdin and environment contain only the fixed note (asserted for the built-in `codex` adapter
      and for a throwaway test adapter).
- [ ] adapter mode in `corvus watch`: exit 0 → delivered; failure/timeout → backoff; missing tool → failure.
- [ ] **a new tool needs no rebuild:** a test drops a new adapter file for an invented tool into the state
      directory, binds an agent to it, and the watcher wakes it through a stub.

Stack commands:
- [ ] `up` without a token starts watchers only and says the HTTP API is disabled; with a token it starts
      `serve` detached and waits for health; `status` reports database, leader, server, watchers; `down` stops
      them; `up` twice is a no-op; stale pid files are handled.
- [ ] by hand: real Claude Code session receives a peer message via the hook; real stop-block happens once.

## Exit criteria

- [ ] `make lint` and `make test` pass.
- [ ] The by-hand check is recorded below with a date.
- [ ] No shell script is required at run time.

## Open questions

- **Codex mechanism.** Expected: `codex exec resume <conversation-id> "<note>"` (documented by OpenAI as of
  2026-11-13). Still to confirm at step 4 on the installed version: the exact syntax; what happens when the
  same conversation is open in an interactive Codex window (does the window see the turn, do the two conflict);
  and which sandbox/approval settings the headless turn runs with, so the nudge cannot do more than the bound
  session could. If the result is unsafe or unusable, the adapter degrades to print-and-notify and the gap is
  documented.
- **Hook contract drift.** Claude Code's hook JSON may have changed since this was written; step 1 settles it.

## Unverified / deferred

—

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0015 | The hook, the supervisor and the stack commands are subcommands of the one binary and open the database directly; the hook is read-only and needs no server or token. | [0015](../decisions/0015-process-topology-single-binary.md) |
| D0021 | Agent tools are supported through declarative adapter files run by one adapter runner; Codex is the first built-in, and tools like OpenCode or Pi can be added later without changing Rust code. No adapter can put message text into a prompt. | [0021](../decisions/0021-agent-adapters.md) |
| D0016 | The supervisor blocks on the same hint-driven `wait_for_inbox` as agents (hints table + `data_version`), so idle waiting costs no tokens and a lost hint only delays delivery. | [0016](../decisions/0016-wakeups-hints-table.md) |
