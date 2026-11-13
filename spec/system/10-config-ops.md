# 10 — Configuration and operations

Corvus runs with **zero configuration**: with no variables set, the binary creates `~/.corvus/corvus.db` and
works. Variables below change defaults or switch on optional surfaces.

## Environment variables

Parsed once at startup into a typed `Config` in `corvus-core`; invalid values fail fast with the variable name.

| Variable | Default | Notes |
|---|---|---|
| `CORVUS_STATE_DIR` | `~/.corvus` | database, hook/watcher state, agent bindings, user adapters, locks, pid and log files |
| `CORVUS_DATABASE_URL` | `<state dir>/corvus.db` | a file path or `sqlite:` URL. Optional (brief §6 says required — [D0013](../decisions/0013-storage-sqlite.md)) |
| `CORVUS_API_TOKEN` | unset | **enables the HTTP API.** Required by `corvus serve`; not used by the desktop app or by agents; ≥ 32 chars; secret. [Why it exists](08-security.md#why-the-http-api-needs-a-token-even-though-it-is-local) |
| `CORVUS_HTTP_PORT` | 8765 | bind is always `127.0.0.1` |
| `CORVUS_LEASE_SECONDS` | 900 | |
| `CORVUS_ACTIVE_SECONDS` | 120 | presence: active window |
| `CORVUS_IDLE_SECONDS` | 900 | presence: offline after this |
| `CORVUS_AUTO_END_SECONDS` | 3600 | 0 disables |
| `CORVUS_OPERATOR_NAME` | `operator` | must satisfy the agent-name grammar |
| `CORVUS_EXPORT_REPO` | unset | absolute path to a git work tree |
| `CORVUS_AGENT_NAME` | unset | default agent name for the MCP process; required by the hook |
| `CORVUS_HOOK_THROTTLE_SECONDS` | 5 | after-tool throttle |
| `CORVUS_ENV_FILE` | `~/.config/corvus/env` | see below |
| `CORVUS_LOG` | `info` | log level |
| `CORVUS_JIRA_*` | | [07a](07a-jira.md#configuration) |
| `CORVUS_LINEAR_*` | | [07b](07b-linear.md#configuration) |

`.env.example` at the repo root lists every variable with placeholder values only. The only secrets are the API
token and the tracker credentials; a default install has none.

## Env file

Processes started by a GUI or by another tool (the desktop app, the hook, an MCP client) do not inherit the
user's shell environment. For them, every Corvus process loads an env file at start:

- `KEY=VALUE` lines, `#` comments; no interpolation, no command substitution.
- Variables already present in the process environment win; the file only fills gaps.
- The loader refuses a file that is group- or world-readable (it must be `0600`) and says so.
- This is the brief's "launcher loads secrets from a local env file", built into the binary: an MCP client
  configuration is just `corvus mcp`, and no secret appears in any client configuration.

## Logging

- `tracing` with a JSON-lines formatter on **stderr** in every process (stdout belongs to CLI output and the MCP
  protocol). Fields: `ts`, `level`, `target`, `msg`, plus span fields (`op`, `session_id`, `task_id`, `agent`,
  `source`, `duration_ms`).
- The desktop app and background `corvus serve` write the same lines to `<state dir>/run/*.log` (rotated by size).
- Never logged: message or task bodies, tokens, `Authorization` headers, tracker credentials. Request logging
  records method, matched route template, status and duration — not query strings.

## Audit events

Every mutating service method inserts an `events` row in its transaction ([03](03-services.md#rules-for-every-service-method)).
Logs are for debugging a process; events are the durable, queryable history shown in the dashboard.

## Background leader

Housekeeping and tracker polling should run in one place even when both `corvus serve` and the desktop app are
open.

- A long-lived process tries to take an exclusive OS file lock on `<state dir>/run/leader.lock`.
- The holder runs the background loops and records its pid and kind in `leader.json` for `corvus status`.
- A non-holder retries every 30 s, so it takes over when the leader exits. The OS releases the lock if the
  leader crashes.
- This is an optimisation, not a safety property: every background step is idempotent, and tracker syncs have
  their own single-flight lock ([07](07-work-sources.md#a-sync-pass)).

## Housekeeping

The leader runs `housekeeping_pass()` every 60 s (first run after 5 s):

```
step "expire_leases"       → huginn.expire_leases()
step "end_stale_sessions"  → sessions.end_stale_sessions()        (skipped when CORVUS_AUTO_END_SECONDS = 0)
step "prune_hints"         → keep the newest 1,000 hint rows
```

- `housekeeping_pass()` is a plain function returning a per-step report; it is callable from tests and from the
  admin routes and commands.
- Each step runs inside its own error boundary (including panics): a failure is logged with the step name and
  the next step still runs. The loop never exits while the process runs (P8).
- The source poll loops ([07](07-work-sources.md#a-sync-pass)) follow the same pattern, one task per enabled source.
- Shutdown: on SIGINT/SIGTERM (or Quit) the process stops accepting connections, lets in-flight requests finish
  (up to 10 s; long polls are woken and return empty), stops the loops, checkpoints the WAL and exits 0.

## Running it

| You want | Do |
|---|---|
| Agents coordinating | Point each agent's MCP config at `corvus mcp`. Nothing else. |
| A dashboard | Open the desktop app (`corvus ui`). |
| The HTTP API for scripts | Set `CORVUS_API_TOKEN`, then `corvus up` (or `corvus serve`). |
| Leases reaped / sessions auto-ended / trackers polled in the background | Keep the desktop app or `corvus serve` running. |
| Idle agents woken | `corvus agent bind <agent> --adapter <tool> …`, then `corvus up` ([06](06-delivery-helpers.md#agent-adapters)). |

Details: [06](06-delivery-helpers.md#stack-commands). Backups: the database is one file; copy it with
`corvus admin backup <path>` (uses `VACUUM INTO`, safe while running) rather than `cp`.
