# Usage reference

Three ways in, one service layer behind them: the **CLI** (`corvus ...`), the **MCP server** (`corvus mcp`) and
the **HTTP API** (`corvus serve`). This page lists every route, command, tool and variable. It is checked against
the code by `crates/corvus-cli/tests/usage_doc.rs`: a route, top-level command, MCP tool or `CORVUS_*` variable
that is not mentioned here fails the test.

Related: [delivery helpers](delivery-helpers.md) (hook, `corvus watch`, adapters) and [releases](release.md).
Design background: [spec/system/04](../spec/system/04-interfaces.md).

| Need | Use | Needs a token or port |
|---|---|---|
| An AI agent talks to Corvus | [MCP server](#mcp-server) | no |
| A shell, script or hook | [CLI](#cli) | no |
| Another program over HTTP, or background loops | [HTTP API](#http-api) via `corvus serve` | yes |

All three open the same SQLite file (default `~/.corvus/corvus.db`) and are safe to use at the same time.

## Contents

- [Environment variables](#environment-variables)
- [Errors and exit codes](#errors-and-exit-codes)
- [CLI](#cli)
- [MCP server](#mcp-server)
- [HTTP API](#http-api)

## Environment variables

Read once at startup by every command. A bad value fails fast, naming the variable (CLI exit code 2). An empty
value counts as unset. Variables in the process environment win over the env file.

| Variable | Default | Rule |
|---|---|---|
| `CORVUS_STATE_DIR` | `~/.corvus` | Absolute path. Created with mode `0700`. Needs `HOME` (absolute) when unset. |
| `CORVUS_DATABASE_URL` | `<state dir>/corvus.db` | Absolute file path, `sqlite:/abs/path` or `sqlite:///abs/path`. No query string, no `:memory:`. |
| `CORVUS_API_TOKEN` | unset | At least 32 characters. Required by `corvus serve` only. Try `openssl rand -hex 32`. |
| `CORVUS_HTTP_PORT` | `8765` | 1-65535. The server always binds `127.0.0.1`; the port is the only knob. |
| `CORVUS_LEASE_SECONDS` | `900` | Task lease length. Whole seconds, 1 to 31536000. |
| `CORVUS_ACTIVE_SECONDS` | `120` | Seen within this long: presence `active`. 1 to 31536000. |
| `CORVUS_IDLE_SECONDS` | `900` | Seen within this long: `idle`; later: `offline`. Must be greater than `CORVUS_ACTIVE_SECONDS`. |
| `CORVUS_AUTO_END_SECONDS` | `3600` | A session silent this long is ended. `0` disables auto-end. 0 to 31536000. |
| `CORVUS_HOOK_THROTTLE_SECONDS` | `5` | Minimum gap between inbox checks by `corvus hook`. `0` disables the throttle. |
| `CORVUS_UPDATE_CHECK` | `on` | `off` stops the desktop app from checking GitHub for a newer version. The `corvus` command never checks. |
| `CORVUS_OPERATOR_NAME` | `operator` | Agent name used for `corvus tell`, `task create` and `task cancel` when no name is given. |
| `CORVUS_AGENT_NAME` | unset | Fixed agent name: used by `corvus mcp` when `register_session` has no name, and by `corvus hook`. |
| `CORVUS_EXPORT_REPO` | unset | Absolute path of the git repository that `admin export` / `export_state_to_git` writes into. |
| `CORVUS_LOG` | `info` | A level (`info`, `debug`, `warn`) or filter made of letters, digits and `_=,:.-`. Logs go to stderr. |
| `CORVUS_ENV_FILE` | `~/.config/corvus/env` | Absolute path of the env file (below). |
| `CORVUS_JIRA_BASE_URL` | unset | `https://` URL, no credentials, query or fragment. Enables Jira only together with the next two. |
| `CORVUS_JIRA_EMAIL` | unset | Jira account email. |
| `CORVUS_JIRA_API_TOKEN` | unset | Jira API token (secret). Setting only some of the three Jira variables is an error. |
| `CORVUS_JIRA_POLL_SECONDS` | `60` | Sync interval. Minimum 15. |
| `CORVUS_JIRA_POOL` | `default` | Pool for tasks created from Jira. At most 64 printable characters. |
| `CORVUS_JIRA_IGNORE_ACCOUNTS` | empty | Comma-separated Jira account ids whose comments are ignored. |
| `CORVUS_LINEAR_API_KEY` | unset | Linear API key (secret). Its presence enables Linear. |
| `CORVUS_LINEAR_POLL_SECONDS` | `60` | Sync interval. Minimum 15. |
| `CORVUS_LINEAR_POOL` | `default` | Pool for tasks created from Linear. |
| `CORVUS_LINEAR_IGNORE_USERS` | empty | Comma-separated Linear user ids whose comments are ignored. |

Agent names (`CORVUS_AGENT_NAME`, `CORVUS_OPERATOR_NAME`) are 1-64 characters of `A-Z a-z 0-9 _ . -`, starting
with a letter or digit. Secrets come only from the environment or the env file; they are never logged and never
shown in `Debug` output.

### Env file

`~/.config/corvus/env` (or `CORVUS_ENV_FILE`) lets `corvus mcp` and the hook run with no secret in any client
configuration.

- One `KEY=VALUE` per line. Blank lines and lines starting with `#` are skipped. No interpolation, no `export`.
- Keys are letters, digits and `_`, not starting with a digit. A matching pair of `"..."` or `'...'` around the
  value is removed.
- A missing file is fine. A file readable or writable by group or others is an error: run `chmod 600 <file>`.
- It only fills gaps: a variable already in the process environment wins.
- A line that is not `KEY=VALUE` fails with the line number.

## Errors and exit codes

Every failure has one shape, whatever the transport: `{"error": {"code": "...", "message": "..."}}`.

| Meaning | `error.code` | HTTP | CLI exit | MCP |
|---|---|---|---|---|
| Not found | `not_found` | 404 | 4 | tool error |
| Conflict (for example a task already claimed) | `conflict` | 409 | 5 | tool error |
| Session ended or unknown; register again | `session_not_active` | 409 | 5 | tool error |
| Invalid input, bad request shape, bad configuration | `validation` | 422 | 2 | tool error |
| Missing or wrong token | `unauthorized` | 401 | 3 | not applicable |
| `Host` header not allowed | `forbidden_host` | 403 | not applicable | not applicable |
| Request body over 256 KiB | `payload_too_large` | 413 | not applicable | not applicable |
| Database write queue timed out; retry | `busy` | 503 | 6 | tool error |
| Anything else (detail is logged, not returned) | `internal` | 500 | 1 | tool error |

Exit code 0 is success. MCP tool errors carry the same JSON as text with `isError: true`.

## CLI

```
corvus [COMMAND]      corvus --help      corvus <command> --help      corvus --version
```

- **Output:** the result as JSON on stdout (pretty when stdout is a terminal, compact otherwise). A command that
  has nothing to print (`mcp`, `serve`) prints nothing.
- **Errors:** the error JSON on stderr and the exit code from the [table above](#errors-and-exit-codes).
  Configuration problems exit 2.
- **Database:** every command opens the database directly and migrates it if needed. No server is involved.
- **Logs:** structured, to stderr, at `CORVUS_LOG`.
- Ids: tasks, threads, sessions and decisions are UUIDs; message ids are integers.

### `corvus session`

| Command | Arguments and flags |
|---|---|
| `corvus session register` | `--agent <NAME>` (omit to generate a name like `keen-raven`), `--client <CLIENT>` (default `cli`), `--metadata <JSON object>`, `--idempotency-key <KEY>` |
| `corvus session heartbeat <SESSION_ID>` | Marks the session alive. |
| `corvus session end <SESSION_ID>` | Ends the session and releases any task it holds. |
| `corvus session list` | `--all` (include ended sessions), `--agent <NAME>`. Includes presence. |

### `corvus connect`

`corvus connect [--agent <NAME>] [--client <CLIENT>]` registers a session and prints it together with the
bootstrap context and the guidance text. Same result as the MCP `register_session` tool.

### `corvus task`

| Command | Arguments and flags |
|---|---|
| `corvus task create` | `--title <TITLE>` (required), `--creator <NAME>` (default `CORVUS_OPERATOR_NAME`), `--body`, `--priority <INT>` (higher first, default 0), `--pool <POOL>` (default `default`), `--recipient <AGENT>` (only that agent may claim), `--depends-on <TASK_ID>` (repeatable), `--idempotency-key`, `--session <SESSION_ID>` |
| `corvus task claim` | `--session <SESSION_ID>` (required), `--pool <POOL>`. Prints `{"task": null, "claim": null}` when nothing is claimable. |
| `corvus task renew <TASK_ID>` | `--session <SESSION_ID>` (required). Extends the lease. |
| `corvus task release <TASK_ID>` | `--session <SESSION_ID>` (required). Gives the task back to the queue. |
| `corvus task complete <TASK_ID>` | `--session <SESSION_ID>` (required), `--summary <TEXT>` |
| `corvus task cancel <TASK_ID>` | `--by <NAME>` (default `CORVUS_OPERATOR_NAME`), `--reason`, `--cascade` (also cancel every dependent), `--session <SESSION_ID>` |
| `corvus task list` | `--status open\|claimed\|done\|cancelled`, `--agent <NAME>` (created, routed to or held by), `--limit <N>`. Newest first. |
| `corvus task get <TASK_ID>` | One task. |

### `corvus msg`, `corvus tell`, `corvus inbox`

Thread ids come from the results of `corvus tell` / `send_direct_message` (the pair's direct thread), `corvus task
create` / `create_task` (the task's thread), `create_thread` (MCP / HTTP), or `corvus inbox read` (each message's
`thread_id`). There is no CLI command that creates a standalone thread. `corvus tell` needs a recipient that
already exists (has registered or been named before); otherwise it fails with `not_found`.

**Broadcasts** (no `--to` / `to`) are unread for every agent except the sender until each acks them; listing views
count the audience as every agent that has ever had a session ([defaults #16](../spec/system/11-defaults.md)).

| Command | Arguments and flags |
|---|---|
| `corvus msg post <BODY>` | `--thread <THREAD_ID>` (required), `--from <NAME>` (default `CORVUS_OPERATOR_NAME`), `--to <NAME>` (omit to broadcast), `--type note\|question\|answer\|handoff\|status\|decision` (default `note`), `--in-reply-to <MESSAGE_ID>`, `--idempotency-key` |
| `corvus tell <AGENT> <MESSAGE>` | Direct message from the operator. `--from <NAME>` (default `CORVUS_OPERATOR_NAME`), `--type <TYPE>` (default `note`). If nothing can deliver to the agent right now (no open session, no waiter, no watcher) the message is still sent and a warning is printed on stderr. |
| `corvus mail reassign <FROM> <TO>` | Operator action: moves `<FROM>`'s unread direct mail to `<TO>`, for a conversation that will not come back. Each message is copied to `<TO>` (same thread, sender, type and text) and the original is marked reassigned, which is not an acknowledgement. Broadcasts are not moved. Running it again moves nothing. Prints `{from, to, moved: [{original_id, message_id}]}`. |
| `corvus inbox read <AGENT>` | Unread messages, oldest first. Never acknowledges. `--limit <N>`, `--after-id <ID>` (paging hint). |
| `corvus inbox ack <MESSAGE_ID>` | `--agent <NAME>` (required). |
| `corvus inbox wait <AGENT>` | Blocks until there is unread mail or the timeout, then prints it. `--timeout <SECONDS>` (default 60, at most 120), `--session <SESSION_ID>`. |

### `corvus digest`, `corvus decision`, `corvus bootstrap`

| Command | Arguments and flags |
|---|---|
| `corvus digest write <SUMMARY>` | `--scope global\|task\|thread\|agent` (required), `--ref <ID-or-NAME>` (omit for `global`), `--author <NAME>`, `--covers-through <MESSAGE_ID>` |
| `corvus digest list` | The latest digest of every scope. |
| `corvus decision record <DECISION>` | `--title <TITLE>` (required), `--rationale`, `--by <NAME>`, `--task <TASK_ID>`, `--thread <THREAD_ID>`, `--supersedes <DECISION_ID>` |
| `corvus decision list` | `--task <TASK_ID>`, `--thread <THREAD_ID>`, `--all` (include superseded). Newest first. |
| `corvus bootstrap <AGENT>` | Routed and claimable tasks, unread messages, open questions, digests and decisions for the agent. |

### Health, database and background

| Command | What it does |
|---|---|
| `corvus health` | Opens (creating and migrating if needed) the database. Prints `{"ok": true, "schema_version": N}`. |
| `corvus migrate` | Same open, and reports `from_version`, `schema_version`, `applied` and any `backup` made. |
| `corvus stats` | Open and claimed tasks, active and expired leases, unread counts. |
| `corvus serve` | The [HTTP API](#http-api) on `127.0.0.1:$CORVUS_HTTP_PORT`, plus the background loops. Exits 2 without `CORVUS_API_TOKEN`. Prints nothing on stdout; logs `serving the HTTP API` with the URL on stderr once bound. Runs until interrupted. |
| `corvus mcp` | The [MCP server](#mcp-server) on stdio. Needs no server, token or port. |
| `corvus ui` | Launches the desktop app, or focuses it if it is running. If it is not installed it says so and exits non-zero; there is no browser fallback. |
| `corvus up` / `corvus down` / `corvus status` | Start, stop and inspect the optional background stack: the HTTP API (when a token is set) and bound watchers. `status` reports the database, background leader (`leader.json`), server and watchers. `server.pid` / `server.running` describe a server started by `corvus up` (it has a pid file — a `corvus serve` started by hand shows up only as the leader); `server.enabled` / `server.healthy` need `CORVUS_API_TOKEN` in the environment of the `status` command. Pid, log and lock files live in `~/.corvus/run/`. |
| `corvus jira-sync` / `corvus linear-sync` | Run one sync pass of that work source now. |

### Admin

| Command | What it does |
|---|---|
| `corvus admin expire-leases` | Ends every expired lease now. Prints `{"expired": N}`. |
| `corvus admin end-stale` | Ends every session silent longer than `CORVUS_AUTO_END_SECONDS`. Prints `{"ended": [ids]}`. |
| `corvus admin export` | Writes the coordination snapshot into `CORVUS_EXPORT_REPO`. `--subdir <DIR>` (default `coordination`), `--commit` (also commit exactly the exported paths). |
| `corvus admin backup <PATH>` | Consistent copy of the database to `PATH`, which must not exist. Safe while Corvus is running. |

### Delivery helpers

Described in [delivery-helpers.md](delivery-helpers.md); listed here for completeness.

| Command | What it does |
|---|---|
| `corvus hook` | The Claude Code inbox hook. Reads the hook event on stdin, prints context or nothing. Never fails: always exits 0, no config errors, no logging unless `CORVUS_LOG` is set. |
| `corvus hook wait` | The background waiter Claude Code runs as an `asyncRewake` hook: blocks until mail arrives for the session's agent, then exits 2 with the mail on stderr, which wakes the idle session. Exits 0 silently otherwise. |
| `corvus setup claude` | Adds the two hooks above to `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR/settings.json`). Prints the change on stderr and asks before writing; leaves `settings.json.corvus-backup`. `--remove` (take the Corvus entries out), `--yes` (do not ask), `--file <PATH>` (another settings file). Prints `{"file", "changed", ...}`. |
| `corvus watch <AGENT>` | Waits for mail for `AGENT` and wakes it or prints it. `--print` (print unread messages as JSON once there are any, then exit; the default for an unbound agent), `--timeout <SECONDS>` (how long a wake command may run, default 300 or the adapter's own), `--exec <CMD> [ARGS...]` (run a command per batch of new mail, never through a shell; must come last). |
| `corvus agent bind <AGENT>` | `--adapter <NAME>` (required), `--conversation <ID>` (the tool's conversation id, if its adapter needs one), `--force` (replace an existing binding). |
| `corvus agent unbind <AGENT>` / `corvus agent list` | Remove a binding / list bindings. |
| `corvus adapter list` | Built-in and user adapters. |
| `corvus adapter show <NAME>` | One parsed adapter. |
| `corvus adapter check <NAME>` | Runs the adapter's check command: is the tool installed? |

## MCP server

`corvus mcp` speaks the Model Context Protocol over stdio. stdout carries protocol only; logs go to stderr. It
opens the database directly, so there is no server, token or port to configure. The server `instructions` are
the Corvus guidance text. Because the binary reads the [env file](#env-file), no secret has to appear in a
client configuration.

### Client configuration

Claude Code:

```sh
claude mcp add corvus -- corvus mcp
```

or in a JSON MCP config:

```json
{"mcpServers": {"corvus": {"command": "corvus", "args": ["mcp"]}}}
```

Codex, in `~/.codex/config.toml`:

```toml
[mcp_servers.corvus]
command = "corvus"
args = ["mcp"]
```

To give the agent a fixed name, set `CORVUS_AGENT_NAME` in the client's environment for the server, for example
`{"mcpServers": {"corvus": {"command": "corvus", "args": ["mcp"], "env": {"CORVUS_AGENT_NAME": "reviewer"}}}}`
in JSON, or an `env = { CORVUS_AGENT_NAME = "reviewer" }` line under `[mcp_servers.corvus]` in TOML. Without it,
`register_session` generates a name like `keen-raven`.

### Session convenience

- The server remembers the session and agent name from the last `register_session`. A later call may omit
  `session_id` and the agent-name field, and the server fills them in. An empty string counts as omitted.
- The remembered session is attached only when the call acts as the remembered agent. A call made explicitly on
  behalf of another agent does not get the remembered `session_id`.
- Before `register_session` nothing is remembered, so `get_bootstrap_context` (no `agent`) fails with a
  `validation` error.
- Tool calls may run concurrently: send `register_session` and **wait for its result** before relying on the
  remembered session in later calls.
- On stdin EOF (the client exits) the server ends the remembered session, best effort, and exits.
- Tool input schemas come from the same request structs the HTTP API accepts, so field names match the
  [HTTP bodies](#http-api). The two ids that are URL path parameters over HTTP (`task_id`, `message_id`) are
  ordinary required arguments here.

### Tools

"Filled in" means the field may be omitted once a session is remembered.

| Tool | Input fields | Filled in from the session |
|---|---|---|
| `register_session` | `agent_name?`, `client_name`, `metadata?`, `idempotency_key?`. Returns the session, bootstrap context and guidance. | `agent_name` falls back to `CORVUS_AGENT_NAME`, then a generated name. Remembers the result. |
| `get_bootstrap_context` | `agent?` | `agent` |
| `create_task` | `creator`, `title`, `body?`, `priority?`, `pool?`, `recipient?`, `depends_on?`, `artifacts?`, `idempotency_key?`, `session_id?` | `creator`, `session_id` |
| `claim_task` | `session_id`, `pool?`. Both result fields are null when nothing is claimable. | `session_id` |
| `renew_lease` | `task_id`, `session_id` | `session_id` |
| `release_task` | `task_id`, `session_id` | `session_id` |
| `complete_task` | `task_id`, `session_id`, `result_summary?` | `session_id` |
| `cancel_task` | `task_id`, `by`, `reason?`, `cascade?`, `session_id?` | `by`, `session_id` |
| `post_message` | `thread_id`, `from`, `body`, `type?` (default `note`), `to?` (omit to broadcast), `in_reply_to?`, `artifacts?`, `idempotency_key?`, `session_id?` | `from`, `session_id` |
| `create_thread` | `creator`, `title`, `task_id?`, `key?`, `session_id?` | `creator`, `session_id` |
| `send_direct_message` | `from`, `to`, `body`, `type?`, `idempotency_key?`, `session_id?` | `from`, `session_id` |
| `read_inbox` | `agent`, `limit?`, `after_id?`. Reading does not acknowledge. | `agent` |
| `wait_for_inbox` | `agent`, `timeout_seconds?` (default 60, clamped to 0-120), `session_id?`, `limit?` | `agent`, `session_id` |
| `ack_message` | `message_id`, `agent`, `session_id?` | `agent`, `session_id` |
| `heartbeat` | `session_id` | `session_id` |
| `end_session` | `session_id` | `session_id` |
| `write_digest` | `author`, `scope` (`global`, `task`, `thread`, `agent`), `ref?`, `summary`, `covers_through?`, `idempotency_key?`, `session_id?` | `author`, `session_id` |
| `record_decision` | `decided_by`, `title`, `decision`, `rationale?`, `task_id?`, `thread_id?`, `supersedes?`, `idempotency_key?`, `session_id?` | `decided_by`, `session_id` |
| `list_decisions` | `task_id?`, `thread_id?`, `include_superseded?`, `limit?` | nothing |
| `export_state_to_git` | `subdir?`, `commit?` | nothing |

An artifact is `{"kind": "file|branch|worktree|commit|pr", "path": "...", "commit?": "...", "description?": "..."}`.
Requests reject unknown fields.

## HTTP API

Start it with `corvus serve` (requires `CORVUS_API_TOKEN`). It is for scripts and other programs; there is no
browser UI, no OpenAPI or schema route, and no pages.

| Property | Behaviour |
|---|---|
| Address | `http://127.0.0.1:$CORVUS_HTTP_PORT` (default 8765). Never any other interface. |
| Auth | `Authorization: Bearer <token>` on **every** route, none exempt. A missing or wrong token gives 401 with `WWW-Authenticate: Bearer`. The token must be at least 32 characters; comparison is constant time. |
| Host allow-list | `Host` must be `127.0.0.1`, `localhost` or `[::1]`, with the configured port or no port. Anything else, or a repeated `Host` header, gets 403 `forbidden_host`. Checked before auth, which defeats DNS rebinding. |
| CORS | None. No CORS headers are sent and preflight requests are not answered, so browsers cannot call the API cross-origin. |
| Body | JSON in and out. POST and PUT bodies need `Content-Type: application/json`. Capped at 256 KiB (413 `payload_too_large`). |
| Request shape | Unknown JSON fields and unknown query parameters are rejected with 422. |
| Path parameters | Validated (agent-name grammar, UUID, integer) before any work happens; a bad one is 422. |
| Long poll | `GET /api/inbox/{agent}/wait`: default 60 s, at most 120 s. The whole-request timeout is 130 s (408). |
| Response headers | `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on every response. |
| Errors | `{"error": {"code": "...", "message": "..."}}` with the status from the [table](#errors-and-exit-codes). An unknown path (404) or wrong method (405) has an empty body. |
| Logs | One line per request: method, route template, status, duration. Never URLs with query strings, headers or bodies. |

### Examples

```sh
export CORVUS_API_TOKEN=$(openssl rand -hex 32)
corvus serve &

# health
curl -s -H "Authorization: Bearer $CORVUS_API_TOKEN" http://127.0.0.1:8765/api/health
# {"ok":true,"schema_version":8}

# create a task
curl -s -X POST http://127.0.0.1:8765/api/tasks \
  -H "Authorization: Bearer $CORVUS_API_TOKEN" -H "Content-Type: application/json" \
  -d '{"creator": "operator", "title": "Write the docs"}'

# wait up to 30 seconds for mail
curl -s -H "Authorization: Bearer $CORVUS_API_TOKEN" \
  "http://127.0.0.1:8765/api/inbox/reviewer/wait?timeout=30"
```

### Routes

Request bodies are the JSON form of the same fields as the CLI flags and [MCP tool inputs](#tools) (the path
parameter replaces the id field). Mutating create routes accept `idempotency_key` (`POST /api/threads` uses `key`);
the same key returns the original result.

**Overview and sessions**

| Route | Request | Response |
|---|---|---|
| `GET /api/health` | none | `{ok, schema_version}` |
| `GET /api/stats` | none | task and lease counts, unread totals and per agent |
| `GET /api/overview` | none | sessions with presence, agents (unread direct mail, reachable, conversations), task columns, stats, recent events and decisions, thresholds |
| `POST /api/sessions` | `{agent_name?, client_name, metadata?, idempotency_key?}` | session |
| `GET /api/sessions` | query `include_ended`, `agent` | list of sessions |
| `POST /api/connect` | as `POST /api/sessions` | `{session, bootstrap, guidance}` |
| `POST /api/sessions/{id}/heartbeat` | none | session |
| `POST /api/sessions/{id}/end` | none | session |

**Tasks**

| Route | Request | Response |
|---|---|---|
| `POST /api/tasks` | `{creator, title, body?, priority?, pool?, recipient?, depends_on?, artifacts?, idempotency_key?, session_id?}` | task |
| `GET /api/tasks` | query `status`, `agent`, `limit` | list of tasks |
| `GET /api/tasks/{id}` | none | task |
| `POST /api/tasks/claim` | `{session_id, pool?}` | `{task, claim}`; status 200 with both `null` when nothing is claimable |
| `POST /api/tasks/{id}/renew` | `{session_id}` | task |
| `POST /api/tasks/{id}/release` | `{session_id}` | task |
| `POST /api/tasks/{id}/complete` | `{session_id, result_summary?}` | task |
| `POST /api/tasks/{id}/cancel` | `{by, reason?, cascade?, session_id?}` | `{task, cancelled: [ids]}` |

**Threads and messages**

| Route | Request | Response |
|---|---|---|
| `POST /api/threads` | `{creator, title, task_id?, key?, session_id?}` | thread |
| `GET /api/threads/{id}` | none | thread |
| `GET /api/threads/{id}/messages` | query `limit`, `after_id` | list of messages |
| `POST /api/messages` | `{thread_id, from, body, type?, to?, in_reply_to?, artifacts?, idempotency_key?, session_id?}` | message |
| `GET /api/messages` | query `limit`, `before_id`, `agent`, `thread`, `unread_only` | messages with `thread_title`, `read_by`, `unread_by` |
| `POST /api/messages/direct` | `{from, to, body, type?, idempotency_key?, session_id?}` | message |
| `POST /api/messages/{id}/ack` | `{agent, session_id?}` | `{message_id, newly_acked}` |
| `GET /api/inbox/{agent}` | query `limit`, `after_id` | list of unread messages; never acknowledges |
| `GET /api/inbox/{agent}/wait` | query `timeout` (seconds, 0-120, default 60), `session_id`, `limit` | list of unread messages; `[]` on timeout |

**Memory and context**

| Route | Request | Response |
|---|---|---|
| `POST /api/digests` | `{author, scope, ref?, summary, covers_through?, idempotency_key?, session_id?}` | digest |
| `GET /api/digests` | none | latest digest of every scope |
| `GET /api/digests/latest` | query `scope` (required), `ref` | digest, or `null` when there is none |
| `GET /api/bootstrap/{agent}` | none | bootstrap context |
| `POST /api/decisions` | `{decided_by, title, decision, rationale?, task_id?, thread_id?, supersedes?, idempotency_key?, session_id?}` | decision |
| `GET /api/decisions` | query `task`, `thread`, `include_superseded`, `limit` | list of decisions |
| `GET /api/events` | query `limit`, `before_id` | audit events, newest first |

**Admin**

| Route | Request | Response |
|---|---|---|
| `POST /api/admin/expire-leases` | none | `{expired}` |
| `POST /api/admin/end-stale-sessions` | none | `{ended: [ids]}` |
| `POST /api/admin/export` | optional `{subdir?, commit?}`; an empty body is fine | `{paths, removed, committed}` |
| `POST /api/admin/reassign-mail` | `{from, to}` | `{from, to, moved: [{original_id, message_id}]}`; operator action, not an MCP tool |

**Work sources** (`{source}` is `jira` or `linear`; see [spec/system/07](../spec/system/07-work-sources.md))

| Route | Request | Response |
|---|---|---|
| `GET /api/sources` | none | `[{source, enabled, site}]` |
| `GET /api/sources/{source}` | none | filter, tickets, known statuses, last sync info |
| `PUT /api/sources/{source}/filter` | `{statuses, done_days, projects, auto_queue}` | source view |
| `POST /api/sources/{source}/sync` | none | `{ok, stats, error}`; a failed pass is reported in the body, not as an HTTP error |
| `POST /api/sources/{source}/queue` | `{key}` | task |

Text that came from Jira or Linear is untrusted input wherever it appears.
