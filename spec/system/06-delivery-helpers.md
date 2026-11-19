# 06 — Delivery helpers

Helpers that make pulling reliable (brief §4.7). None of them acks: only the agent acknowledges a message.
All are subcommands of the one `corvus` binary ([D0015](../decisions/0015-process-topology-single-binary.md));
none needs a server. Support for a particular agent tool is added through an [adapter](#agent-adapters).

## Inbox hook (`corvus hook`)

Reads the database directly through a single **read-only** connection. It needs no token, no port and no
running service, and never writes to the database.

**Contract with Claude Code** (confirmed 2026-11-13 against the Claude Code hooks reference, Claude Code 2.1.323):
registered for `UserPromptSubmit`, `PostToolUse` and `Stop`; reads the event JSON on stdin (`hook_event_name`,
`session_id`, `stop_hook_active`); writes JSON on stdout — `{"hookSpecificOutput": {"hookEventName": …,
"additionalContext": …}}` for `UserPromptSubmit` and `PostToolUse` (plain stdout reaches the model only for
`UserPromptSubmit`, so JSON is used for both), `{"decision": "block", "reason": …}` for `Stop`. Empty stdout on
exit 0 is a no-op; exit 2 is never used. Claude Code moves context over 10,000 characters to a file, so the
injected text is kept under 9,000. Settings `env` is not documented to reach hooks, so the agent name is set in
the hook command ([docs](../../docs/delivery-helpers.md)).

| Event | Behaviour |
|---|---|
| `UserPromptSubmit` | Read unread; inject new ones as additional context. |
| `PostToolUse` | Same, but skipped if the last check was < `CORVUS_HOOK_THROTTLE_SECONDS` (default 5) ago. |
| `Stop` | If `stop_hook_active` is true → do nothing (no loops). Else if new mail is waiting → block stopping once, with the formatted messages as the reason. |

Rules:

- **Fast path**: the throttle check reads only the state file, before the database is opened. The hook does not
  start an async runtime, does not migrate, and does not initialise logging unless `CORVUS_LOG` asks for it.
  Budget: 50 ms typical; a 300 ms busy timeout on the read.
- **Inputs**: `CORVUS_AGENT_NAME` (optional, see [Which agent](#which-agent)), `CORVUS_STATE_DIR`,
  `CORVUS_DATABASE_URL`, `CORVUS_OPERATOR_NAME`, from the environment and the env file
  ([10](10-config-ops.md#env-file)).
- **Dedupe**: state file `$CORVUS_STATE_DIR/hook/<agent>__<claude-session-id>.json` holds
  `{injected_ids: [...], last_check: <ts>, stop_blocked: bool}`. A message id is injected at most once per
  (agent, Claude session). Written with temp-file + rename. The injected set is pruned to ids still unread.
- **Format**: the guidance text, then up to 10 messages (oldest first): id, thread, sender, type, body truncated
  to ~1000 chars with a marker. The sender equal to `CORVUS_OPERATOR_NAME` is shown as "the operator (your
  user)", and the header says operator mail is the user speaking while mail from other agents is a peer request
  ([D0028](../decisions/0028-operator-mail-is-the-owner.md)). If more than 10 are new, say how many remain.
  Bodies are quoted text, never merged into the instructions.
- **Never fails**: every error path (no database, schema mismatch, busy, bad JSON, unwritable state, panic via
  `catch_unwind`) exits 0 with empty output. The hook never acks. A corrupt state file is reset (silent that
  once); if the state cannot be saved the hook prints nothing, so it never re-injects the same mail every call.
- State-file and agent-name components are validated before they are used in a path.

### Which agent

Decision: [D0023](../decisions/0023-agent-identity-follows-conversation.md).

1. `CORVUS_AGENT_NAME`, if set (a name written into a project's hook command — the original setup).
2. Otherwise the agent linked to the event's `session_id` (`agent_for_conversation('claude-code', id)`,
   [03](03-services.md#sessions-and-presence)).
3. Otherwise nothing: exit 0 silently. A session that never registered with Corvus is left alone, which is what
   makes a **user-level** hook safe for every session on the machine.

### Session events

Two more events are registered so identity survives `/clear` (measured on Claude Code 2.1.325: a clear fires
`SessionEnd` with `reason: "clear"` and the old id, then `SessionStart` with `source: "clear"` and a new id, in
the same process):

| Event | Behaviour |
|---|---|
| `SessionEnd`, `reason = clear` | Write `$CORVUS_STATE_DIR/hook/clear/<claude-pid>.json` = `{from: <old id>, at: <ts>}`. Other reasons: nothing. |
| `SessionStart`, `source = clear` | If that note exists and is under 60 s old: `carry_conversation('claude-code', from, <new id>)`, then delete the note. |
| `SessionStart`, any source | Write `$CORVUS_STATE_DIR/hook/current/<claude-pid>.json` = `{conversation: <id>}` (read by the waiter). |

The carry-over is the **only** database write the hook ever makes. It opens the database for writing for that
one event, never migrates (a schema mismatch is a silent skip), and keeps the never-fails rule. Every other
event stays on the read-only connection. `<claude-pid>` is `CLAUDE_PID`, else the hook's parent process id (the
hook is a direct child of `claude`).

## Waiter (`corvus hook wait`)

Wakes an idle Claude Code session when mail arrives. Decision:
[D0024](../decisions/0024-wake-through-the-tools-own-channel.md). Claude Code runs a hook marked
`"asyncRewake": true` in the background and, when it exits with code 2, starts a turn with the hook's stderr as
the prompt. The waiter is that hook: it blocks on the inbox the way `wait_for_inbox` does (no model tokens),
on the read-only connection.

Registered on `SessionStart` and `Stop` with `"timeout": 86400` (the default is 600 s, after which Claude Code
kills the hook and the session can no longer be woken).

- **One waiter per Claude Code process**: an exclusive file lock on
  `$CORVUS_STATE_DIR/hook/wait/<claude-pid>.lock`. Every stop starts another waiter; one that finds the lock
  held exits 0 at once.
- **It follows the process's current conversation** (`hook/current/<claude-pid>.json`), re-reading it each
  tick, and resolves the agent from that ([Which agent](#which-agent)). So it survives `/clear` and an
  in-session `/resume` without a second waiter. No agent yet → it keeps waiting and looks again every 5 s (the session may register later).
- **What wakes**: an unread message addressed to the agent, or a broadcast the operator marked `wake`, whose
  id is not in the hook's shown-set. An ordinary broadcast is shown at the session's next turn by the inbox
  hook, not by a wake ([11](11-defaults.md) row 62).
- **On new mail**: add the ids to the shown-set, save it, print the same text the `Stop` block prints (guidance
  + messages) to stderr, exit 2. If the state cannot be saved it exits 0 instead — never a wake loop. A wake
  arrives as a prompt, so `UserPromptSubmit` then runs the inbox hook; the shared shown-set stops a repeat.
- **Exits 0, silently**, when: the session is headless (`CLAUDE_CODE_ENTRYPOINT` is set and is not `cli` — a
  waiter keeps `claude -p` from ever exiting); its parent process is gone (checked at least once a second; a
  killed session orphans its hooks); it receives `SIGTERM`/`SIGHUP` (Claude Code's normal shutdown and its
  timeout); or on any error (no database, schema mismatch, bad input).
- A wake that arrives during a turn is queued by Claude Code until the turn ends; one that arrives while the
  user is typing leaves the typed text in place (both measured).

**Reachable.** An agent is *reachable* when it has a session that has not ended, or a waiter holds the lock
for a process whose current conversation resolves to it, or a watcher holds `watch/<agent>/lock`. `corvus tell`
to an agent that is not reachable still delivers, and warns: the message waits until that conversation is
resumed. The dashboard flags agents that have unread direct mail and are not reachable, shows each
conversation's directory, and offers **Reassign** to move that mail to another agent
([D0025](../decisions/0025-operator-reassigns-stranded-mail.md), [05](05-dashboard.md)).

**Setup** is one user-level file, `~/.claude/settings.json`: `corvus hook` on `SessionStart`, `SessionEnd`,
`UserPromptSubmit`, `PostToolUse`, `Stop`, and `corvus hook wait` (with `asyncRewake` and the timeout) on
`SessionStart` and `Stop`. A running session picks the entries up at its next event. Two ways, same result:

- `corvus setup claude` prints the exact change (a line diff, on stderr) and asks before writing; `--yes` skips
  the question, `--remove` takes the entries out again, `--file <path>` edits another settings file (a
  project's), and `CLAUDE_CONFIG_DIR` moves the default as it does for Claude Code.
  - It edits only its own entries, recognised by their command: a program named `corvus` followed by exactly
    `hook` or `hook wait`, with nothing in front. A hand-written `CORVUS_AGENT_NAME=x corvus hook` is not its
    entry and stays.
  - The entries use the **absolute path of the binary that ran the command** (double-quoted if it needs it):
    hooks do not reliably see the user's `PATH`. Running it again after the binary moved replaces them.
  - Everything else keeps its content and key order; the file is printed with two-space indentation. A file
    that already has the entries is not written at all. Removing gives back what was there.
  - It refuses a file that is not valid JSON or whose `hooks` has an unexpected shape, writes through a temp
    file and a rename (through a symlink, not over it), re-reads the file before writing and stops if it
    changed while the question was open, and leaves a `settings.json.corvus-backup` beside it.
  - It does not touch `permissions`. It prints the rule to add by hand — `"mcp__corvus"` in
    `permissions.allow` — because a woken turn otherwise stops at the tool-permission prompt.
  - The edit is a pure function in `corvus-core` (`delivery::claude_setup`); the CLI reads, asks and writes.
    The desktop app's "Connect an agent" snippet comes from the same function.
- The same snippet in the [docs](../../docs/delivery-helpers.md), for pasting by hand.

## Wake supervisor (`corvus watch <agent>`)

Blocks on the inbox with `wait_for_inbox` (no model tokens spent).

- **Adapter mode** (default for a bound agent): for each batch of new unread messages **that wake** — mail
  addressed to the agent and broadcasts marked `wake` ([11](11-defaults.md) row 62) — run the agent's adapter
  ([below](#agent-adapters)). Other broadcasts stay unread for the agent's next turn. Same delivered-set, retry
  and timeout rules as exec mode.
- **Print mode** (`--print`, and the default for an unbound agent): when unread messages exist, print them as
  JSON and exit 0.
- **Exec mode** (`--exec <cmd>`): loop forever. For each batch of **new** unread messages (ids not in the
  delivered set) run the command with:
  `CORVUS_WAKE_AGENT=<agent>`, `CORVUS_WAKE_STATE_DIR=<dir>`, `CORVUS_WAKE_MESSAGES_FILE=<path to JSON>`, and
  the same JSON on stdin. The command is spawned from an argument vector, never through a shell.
  - Exit 0 → **delivered**: add the batch ids to the delivered set. Delivered is not handled; the supervisor
    never acks.
  - Non-zero exit or timeout (`--timeout`, default 300 s; the process group is killed) → failure: retry the
    same batch with exponential backoff 1 s, 2 s, 4 s … capped at 300 s. Backoff resets on success.
- **Delivered set**: `$CORVUS_STATE_DIR/watch/<agent>/delivered.json` — a *set* of ids, not a high-water mark
  (brief §4.7). Persisted with temp-file + rename after each delivery; pruned to ids that are still unread.
- **One watcher per agent**: an exclusive OS file lock on `$CORVUS_STATE_DIR/watch/<agent>/lock`, held for the
  process lifetime. A second watcher exits with a clear message.
- **Crash semantics**: a crash between the command succeeding and the set being persisted re-delivers that
  batch after restart. Duplicate delivery is acceptable and documented; lost delivery is not.
- Restart resumes from the persisted set; messages that arrived while down are delivered on startup.

## Agent adapters

How Corvus supports a specific agent tool. Decision: [D0021](../decisions/0021-agent-adapters.md).

Two separate things, per tool:

| | How | Per-tool work |
|---|---|---|
| **Connect** — the agent uses Corvus | Any tool that speaks MCP runs `corvus mcp`. Others can call the `corvus` CLI. | none in Corvus; a config snippet in the docs / app |
| **Wake** — an idle agent is told it has mail | The wake supervisor runs the tool's **adapter** | one adapter file |

### Adapter file

TOML. Built-ins are embedded in the binary; user adapters live in `$CORVUS_STATE_DIR/adapters/<name>.toml` and
override a built-in of the same name.

```toml
name  = "codex"
title = "OpenAI Codex CLI"

[binding]                       # what `corvus agent bind` must be given for this tool
conversation = { required = true, pattern = "uuid" }      # "uuid" or "chars:<allowed>:<min>-<max>"

[wake]                          # run when the bound agent has new unread messages
command = ["codex", "queue", "--thread", "{conversation}", "--resume", "--message", "{note}"]
timeout_seconds = 30

[check]                         # optional: is the tool installed?
command = ["codex", "--version"]
```

Rules, enforced by the one adapter runner for every tool:

- **Placeholders** are a closed set: `{agent}`, `{conversation}`, `{note}`, `{count}`, `{state_dir}`. Each
  must be a whole argument. An unknown placeholder makes the adapter invalid at load time.
- **There is no placeholder for message text.** `{note}` is the fixed constant `corvus_domain::WAKE_NOTE`:
  *"You have unread Corvus messages. Check your Corvus inbox (read_inbox), handle them, and ack each one."*
  Peer message text can never reach a tool's prompt through an adapter.
- The command is spawned from the argument list, never through a shell; the process group is killed on timeout.
- Exit 0 = **delivered** (the tool accepted the nudge), not acknowledged. Anything else is a failure and the
  supervisor retries with backoff. A missing tool is a failure with a clear log line.
- Agent names and binding values are validated (agent-name grammar; the adapter's `pattern`) before use.
- An adapter file is loaded only if it is a regular file (not a symlink), owned by the same user as the state
  directory, and not group- or world-writable; an invalid user adapter is an error, never a silent fall-back to a
  built-in.
- Binding patterns are `"uuid"` or `"chars:<allowed>:<min>-<max>"` (a set of allowed characters and a length
  range) rather than regular expressions: no regex engine in the pure domain crate, and a pattern cannot be
  written that matches more than intended.

### Bindings

`corvus agent bind <agent> --adapter <name> [--conversation <id>]` records, in
`$CORVUS_STATE_DIR/agents/bindings.json`, which tool an agent runs in and the identifier needed to wake it. One
binding per agent; re-binding requires `--force`. `corvus agent list` / `unbind` manage them;
`corvus adapter list` / `show <name>` / `check <name>` inspect adapters.

### Bound from the conversation

An agent with no entry in `bindings.json` is treated as bound when its newest conversation link
([D0023](../decisions/0023-agent-identity-follows-conversation.md)) is for a tool that has an adapter of the
same name: adapter = the tool, `{conversation}` = that conversation id. An explicit `corvus agent bind` wins.
`claude-code` has no adapter — its sessions wake themselves through the [waiter](#waiter-corvus-hook-wait).

**Who runs it.** Once the desktop app is running the owner should need nothing else, so the background leader
(the app or `corvus serve`, [10](10-config-ops.md#background-leader)) supervises the agents bound from a
conversation: a wake step in its loop runs the adapter for new mail, with the same delivered-set, back-off and
per-agent lock as `corvus watch` (so the two never wake the same agent twice). `corvus up` and `corvus watch`
remain for explicit bindings and headless machines. With no leader running, those sessions are not woken; they
see their mail at their next turn if their tool has a hook.

### Built-in: `opencode`

- `["opencode", "run", "--session", "{conversation}", "{note}"]`; `{conversation}` pattern
  `chars:A-Za-z0-9_:8-64` (ids look like `ses_…`). OpenCode 2.x only.
- Measured 2026-11-16 on 2.3.0: this starts a turn in an idle session through OpenCode's shared background
  service, and a window open on that session shows the turn live.
- The command returns when the turn ends; its output is discarded. `timeout_seconds = 3600` (the maximum),
  because the turn runs in OpenCode's service, not in the command: killing the command mid-turn does not stop
  the turn (measured 2026-11-16), so a timeout would only queue a second wake for mail that is still unread.
- The leader re-reads the set of conversation-bound agents every 5 s, so a new session is wakeable a few
  seconds after it registers.

### Built-in: `codex`

- `["codex", "queue", "--thread", "{conversation}", "--resume", "--message", "{note}"]`; `{conversation}` is the
  Codex thread id (a UUID). Needs Codex ≥ 0.157 (`codex queue`); measured 2026-11-16 on 0.169.0.
- A session open in a window runs the queued note as a turn a few seconds later and shows it live. With no
  window open, `--resume` has Codex's own background service run the turn headless.
- The command returns at once: exit 0 means *queued*, not handled. `timeout_seconds = 30`.
- The conversation link comes from `register_session` ([04](04-interfaces.md)); Codex has hooks in Claude
  Code's format, but `corvus hook` does not serve them yet, so a Codex agent sees mail when woken or when it
  reads its inbox.
- Codex must be signed in, have the Corvus MCP server in `~/.codex/config.toml`, and have its tools
  pre-approved (`default_tools_approval_mode = "approve"`), or a woken turn cannot read the inbox.

### Adding a tool later (for example OpenCode or Pi)

1. Find out how that tool can be given a prompt for an existing session from the command line.
2. Write its adapter file; try it with `corvus adapter check` and `corvus agent bind`.
3. To ship it as a built-in: add the file to the embedded set, with a test that runs it against a stub of the
   tool's CLI.

No Rust changes are needed for steps 1–2. A tool with no command-line way to resume a session cannot be woken;
its agents still work and rely on their own inbox checks. `corvus watch --exec <cmd>` remains for anything a
template cannot express.

## Stack commands

The brief's convenience scripts are subcommands, because a binary distribution cannot rely on shipped shell
scripts ([11](11-defaults.md)). The `Makefile` has matching `up` / `down` / `status` targets for development.

| Command | Does |
|---|---|
| `corvus up` | ensure the state directory and database exist → migrate → start `corvus serve` in the background with a pid file and a log file (only if a token is configured) → wait for it to answer `health` → start every bound watcher |
| `corvus down` | stop watchers → stop the background server (pid file). The database is untouched. |
| `corvus status` | database path and schema version, background leader (who holds the lock), server pid + health, watchers per agent |

- "Start storage" from the brief is "create the file and migrate" — there is no storage service.
- "Bound watchers" = every agent with a binding (`corvus agent bind`) or bound from its conversation
  ([above](#bound-from-the-conversation)). Pid and log files live under `$CORVUS_STATE_DIR/run/`.
- All three are idempotent: `up` on a running stack is a no-op that reports status.
- Background processes are started detached (new session), with stdout/stderr to the log file.
- None of this is required for agents; it exists for the HTTP API, the background loops and the watchers.
