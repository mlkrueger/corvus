# 06 — Delivery helpers

Helpers that make pulling reliable (brief §4.7). None of them acks: only the agent acknowledges a message.
All are subcommands of the one `corvus` binary ([D0015](../decisions/0015-process-topology-single-binary.md));
none needs a server. Support for a particular agent tool is added through an [adapter](#agent-adapters).

## Inbox hook (`corvus hook`)

Reads the database directly through a single **read-only** connection. It needs no token, no port and no
running service, and never writes to the database.

**Contract with Claude Code** (to be re-verified against current Claude Code hook docs at implementation):
registered for `UserPromptSubmit`, `PostToolUse` and `Stop`; reads the event JSON on stdin (`hook_event_name`,
`session_id`, `stop_hook_active`); writes JSON on stdout.

| Event | Behaviour |
|---|---|
| `UserPromptSubmit` | Read unread; inject new ones as additional context. |
| `PostToolUse` | Same, but skipped if the last check was < `CORVUS_HOOK_THROTTLE_SECONDS` (default 5) ago. |
| `Stop` | If `stop_hook_active` is true → do nothing (no loops). Else if new mail is waiting → block stopping once, with the formatted messages as the reason. |

Rules:

- **Fast path**: the throttle check reads only the state file, before the database is opened. The hook does not
  start an async runtime, does not migrate, and does not initialise logging unless `CORVUS_LOG` asks for it.
  Budget: 50 ms typical; a 300 ms busy timeout on the read.
- **Inputs**: `CORVUS_AGENT_NAME` (required; absent → exit 0 silently), `CORVUS_STATE_DIR`,
  `CORVUS_DATABASE_URL`, `CORVUS_OPERATOR_NAME`, from the environment and the env file
  ([10](10-config-ops.md#env-file)).
- **Dedupe**: state file `$CORVUS_STATE_DIR/hook/<agent>__<claude-session-id>.json` holds
  `{injected_ids: [...], last_check: <ts>, stop_blocked: bool}`. A message id is injected at most once per
  (agent, Claude session). Written with temp-file + rename. The injected set is pruned to ids still unread.
- **Format**: the guidance text, then up to 10 messages (oldest first): id, thread, sender, type, body truncated
  to ~1000 chars with a marker. The sender equal to `CORVUS_OPERATOR_NAME` is shown as "the operator". If more
  than 10 are new, say how many remain. Bodies are framed as quoted peer text, not instructions.
- **Never fails**: every error path (no database, schema mismatch, busy, bad JSON, unwritable state, panic via
  `catch_unwind`) exits 0 with empty output. The hook never acks.
- State-file and agent-name components are validated before they are used in a path.

## Wake supervisor (`corvus watch <agent>`)

Blocks on the inbox with `wait_for_inbox` (no model tokens spent).

- **Adapter mode** (default for a bound agent): for each batch of new unread messages, run the agent's
  adapter ([below](#agent-adapters)). Same delivered-set, retry and timeout rules as exec mode.
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
conversation = { required = true, pattern = "uuid" }      # "uuid" or a regular expression

[wake]                          # run when the bound agent has new unread messages
command = ["codex", "exec", "resume", "{conversation}", "{note}"]
timeout_seconds = 300

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
- An adapter file is loaded only if it is owned by the user and not group- or world-writable.

### Bindings

`corvus agent bind <agent> --adapter <name> [--conversation <id>]` records, in
`$CORVUS_STATE_DIR/agents/bindings.json`, which tool an agent runs in and the identifier needed to wake it. One
binding per agent; re-binding requires `--force`. `corvus agent list` / `unbind` manage them;
`corvus adapter list` / `show <name>` / `check <name>` inspect adapters.

### Built-in: `codex`

- Wakes by resuming a stored Codex conversation headlessly — `codex exec resume <conversation-id> "<note>"`.
  OpenAI's non-interactive docs describe resuming a specific session by id (checked 2026-11-13); the exact
  flags are confirmed against the installed version in [plan 0007](../plans/0007-delivery-helpers.md).
- This adds a new turn to the stored conversation. It does **not** type into a Codex window that is open in a
  terminal; behaviour with an open window is checked in plan 0007.
- Codex must be installed and signed in.

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
- "Bound watchers" = every agent with a binding (`corvus agent bind`). Pid and log files live under `$CORVUS_STATE_DIR/run/`.
- All three are idempotent: `up` on a running stack is a no-op that reports status.
- Background processes are started detached (new session), with stdout/stderr to the log file.
- None of this is required for agents; it exists for the HTTP API, the background loops and the watchers.
