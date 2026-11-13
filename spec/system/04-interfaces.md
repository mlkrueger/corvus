# 04 — Interfaces

Three programmatic transports over one service layer (brief §5), plus the desktop app's IPC carrier, which
reuses the HTTP route table. The dashboard is in [05](05-dashboard.md).
A transport parses input, calls exactly one service method, and serialises the result. Nothing else.

## Error model

| `CorvusError` | HTTP | `error.code` | MCP | CLI exit |
|---|---|---|---|---|
| `NotFound` | 404 | `not_found` | tool error | 4 |
| `Conflict` | 409 | `conflict` | tool error | 5 |
| `SessionNotActive` | 409 | `session_not_active` | tool error | 5 |
| `Validation` (domain or request shape) | 422 | `validation` | tool error | 2 |
| `Unauthorized` | 401 | `unauthorized` | n/a | 3 |
| host not allowed | 403 | `forbidden_host` | n/a | n/a |
| `Busy` (database write queue timed out; retry) | 503 | `busy` | tool error | 6 |
| `Internal` | 500 | `internal` (no detail leaked) | tool error | 1 |

Body shape for every error raised by a handler or a middleware: `{"error": {"code": "...", "message": "..."}}`.
MCP tool errors return the same JSON as text with `isError: true`, so the agent can read the message (for example
"register again"). HTTP specifics (as built):

| Case | Response |
|---|---|
| unknown path / wrong method on a known path | `404` / `405` with an empty body (plus the security headers) |
| body over 256 KiB | `413` `payload_too_large` |
| whole request over 130 s (only a stuck request; the long poll ends at 120 s) | `408` |
| missing or wrong token | `401` `unauthorized` with `WWW-Authenticate: Bearer` |
| POST/PUT without `Content-Type: application/json`, malformed JSON, unknown field | `422` `validation` |

## HTTP API

`corvus-http` exposes two things:

- `data_router()` — the **inner** router: every `/api/*` route, handlers only. No authentication. It is never
  bound to a socket directly; the desktop app calls it in memory ([05](05-dashboard.md#desktop-app-cratescorvus-desktop-tauri-2)).
- `serve()` — the **outer** application: host allow-list → bearer auth → body limit → timeout → logging, wrapped
  around `data_router()`. This is the only thing ever bound, and it serves no pages.

The listener exists only in `corvus serve`, which requires `CORVUS_API_TOKEN` and will not start without it
([why](08-security.md#why-the-http-api-needs-a-token-even-though-it-is-local)). It is a programmatic interface
for scripts and other tools; there is no browser UI and no `/ui` route ([D0020](../decisions/0020-desktop-only-dashboard.md)).
Base `http://127.0.0.1:${CORVUS_HTTP_PORT}`. JSON in and out. `Authorization: Bearer <token>` on **every**
route — none is exempt. There is no OpenAPI, schema or docs route (P6). Request bodies are capped at 256 KiB.

Create routes take `idempotency_key` in the JSON body (sessions / connect, tasks, messages, direct messages,
digests, decisions); threads use their own `key`. The other mutations (renew, release, complete, cancel, ack,
admin, sources) are idempotent by their own rules and take no key. Full request/response shapes and every query
parameter (e.g. `limit`, `after_id`, `include_ended`) are in [docs/usage.md](../../docs/usage.md), which a test
keeps in step with the route table.

| Method & path | Service call |
|---|---|
| `GET /api/health` | `health` |
| `GET /api/stats` · `GET /api/overview` | `stats` · `overview` |
| `POST /api/sessions` · `GET /api/sessions` | `register_session` · list (with presence) |
| `POST /api/connect` | `connect` |
| `POST /api/sessions/{id}/heartbeat` · `POST /api/sessions/{id}/end` | `heartbeat` · `end_session` |
| `POST /api/tasks` · `GET /api/tasks?status=&agent=&limit=` · `GET /api/tasks/{id}` | `create_task` · `list_tasks` · `get_task` |
| `POST /api/tasks/claim` | `claim_task` (`{session_id, pool?}`; `200` with `null` task when none) |
| `POST /api/tasks/{id}/renew` · `/release` · `/complete` · `/cancel` | `renew_lease` · `release_task` · `complete_task` · `cancel_task` |
| `POST /api/threads` · `GET /api/threads/{id}` · `GET /api/threads/{id}/messages` | `create_thread` · get · per-thread list |
| `POST /api/messages` · `GET /api/messages?limit=&before_id=&agent=&thread=&unread_only=` | `post_message` · `list_messages` |
| `POST /api/messages/direct` | `send_direct` |
| `GET /api/inbox/{agent}?limit=&after_id=` | `read_inbox` |
| `GET /api/inbox/{agent}/wait?timeout=&session_id=` | `wait_for_inbox` (long poll, ≤ 120 s) |
| `POST /api/messages/{id}/ack` | `ack_message` (`{agent, session_id?}`) |
| `POST /api/digests` · `GET /api/digests` · `GET /api/digests/latest?scope=&ref=` | `write_digest` · `list_latest_digests` · `latest_digest` |
| `GET /api/bootstrap/{agent}` | `bootstrap` |
| `POST /api/decisions` · `GET /api/decisions?task=&thread=&include_superseded=` | `record_decision` · `list_decisions` |
| `GET /api/events?limit=&before_id=` | list audit events |
| `POST /api/admin/expire-leases` · `/end-stale-sessions` · `/export` | `expire_leases` · `end_stale_sessions` · `export_state_to_git` |
| `GET /api/sources` · `GET /api/sources/{source}` · `PUT …/filter` · `POST …/sync` · `POST …/queue` | work sources (`jira`, `linear`) — [07](07-work-sources.md#http-surface) |

Path parameters are validated (agent-name grammar, UUID, integer) before the service is called; failures are 422.
Response headers on every route: `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`.
Handlers are async; each calls the synchronous service through the `Corvus` facade
([01](01-architecture.md#database-access)).

## MCP server (`corvus mcp`, stdio)

- Built with `rmcp`. stdout carries protocol only; logs go to stderr.
- Opens the database directly. **Needs no server, token or port** — a client configuration is just the command
  `corvus mcp`.
- Server `instructions` = `corvus_domain::GUIDANCE`, verbatim.
- Tools (exactly brief §5): `register_session` (returns the full `connect` result), `get_bootstrap_context`,
  `create_task`, `claim_task`, `renew_lease`, `release_task`, `complete_task`, `cancel_task`, `post_message`,
  `create_thread`, `send_direct_message`, `read_inbox`, `wait_for_inbox`, `ack_message`, `heartbeat`,
  `end_session`, `write_digest`, `record_decision`, `list_decisions`, `export_state_to_git`.
- Tool input schemas are derived from the same request structs the HTTP handlers deserialise (one definition of
  each request shape, in `corvus-core::api`).
- Session convenience: the process remembers the session it registered and fills in `session_id` (and the
  agent name) when a tool call omits them. If `CORVUS_AGENT_NAME` is set, `register_session` without a name uses
  it instead of generating one.
- On stdin EOF the process ends its remembered session (best effort) and exits.
- **Launcher**: the brief's launcher script is unnecessary — the binary loads the env file itself
  ([10](10-config-ops.md#env-file)), so no secret appears in any client configuration.

## CLI (`corvus`)

JSON on stdout (pretty when stdout is a TTY, compact otherwise); errors as the error JSON on stderr with the
exit codes above. Every command opens the database directly ([D0015](../decisions/0015-process-topology-single-binary.md)).

```
corvus session register|heartbeat|end|list
corvus connect
corvus task create|claim|renew|release|complete|cancel|list|get
corvus msg post
corvus inbox read|ack|wait
corvus tell <agent> <message>          # operator direct message; --from defaults to CORVUS_OPERATOR_NAME
corvus digest write|list
corvus decision record|list
corvus bootstrap <agent>
corvus stats | health
corvus migrate
corvus serve                           # HTTP API + background loops; requires CORVUS_API_TOKEN; prints nothing
corvus mcp                             # stdio; stdout is protocol only
corvus ui                              # launches (or focuses) the desktop app
corvus jira-sync | linear-sync
corvus admin expire-leases|end-stale|export|backup
corvus hook                            # Claude Code inbox hook; always exits 0     (06)
corvus watch <agent> [--exec <cmd>]    # wake supervisor            (06)
corvus agent bind|unbind|list         # which tool an agent runs in (06)
corvus adapter list|show|check        # agent-tool adapters        (06)
corvus up | down | status              # stack commands             (06)
```

`corvus ui` launches the desktop app, or focuses it if it is already running. If the app is not installed it
says so and exits non-zero; there is no browser fallback.

## Desktop IPC

The desktop app's frontend sends `(method, path, body)` through one Tauri command into `data_router()`. It is a
carrier for the HTTP API above, not a separate interface: same paths, same bodies, same errors
([D0020](../decisions/0020-desktop-only-dashboard.md)).
