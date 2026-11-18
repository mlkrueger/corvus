# 08 — Security

Controls for P6 and P7. Corvus is a single-user, single-machine tool; the design goal is that nothing off the
machine, and no web page in the user's browser, can read or change its state.

## Surfaces

| Surface | Exists when | Protected by |
|---|---|---|
| Database file | always | OS file permissions (`0700` directory, `0600` files), owned by the user |
| Loopback HTTP API (programmatic; no pages) | only while `corvus serve` runs, which requires `CORVUS_API_TOKEN` | bind, token on every route, host allow-list (below) |
| Desktop app IPC | while the app runs | only the app's own bundled page can call it; one data command |
| Outbound to Linear / Jira | only if configured | fixed hosts, TLS, no redirects |
| Outbound to GitHub Releases (update check) | while the desktop app runs, unless `CORVUS_UPDATE_CHECK=off` | fixed HTTPS URL; an update is installed only on the owner's click and only if its signature matches the updater key ([D0027](../decisions/0027-app-updates-from-github-releases.md)) |

**A default install has no network surface at all, and the desktop app never has one.** (A surface is
something that accepts connections; the update check is an outbound request.)

## Why the HTTP API needs a token even though it is local

"Local" is not "private". A port on `127.0.0.1` is reachable by anything running on the machine:

- **Web pages in the user's browser.** Any site can make the browser send requests to
  `http://127.0.0.1:<port>`. The page usually cannot read the reply, but the request arrives and takes effect.
- **Any other process or OS user.** A loopback port has no file permissions; anyone on the machine can connect.

This matters more for Corvus than for most local tools: tasks and messages are read and acted on by coding
agents that can run commands and edit code. Whoever can write to Corvus can, in effect, instruct those agents.
The token is the only thing that proves a request comes from the operator.

Nothing else needs a token, because everything else goes through the database file, which ordinary OS
permissions already restrict to the user. The CLI, `corvus mcp`, the hook and the desktop app never use a port.
The token exists only because the HTTP API replaces file permissions with a network socket.

**Rule (owner-confirmed, 2026-11-13): no token, no start.** `corvus serve` refuses to run without
`CORVUS_API_TOKEN`, including when it is wanted only for the headless background loops. There is no
token-less mode.

## Threats and controls

| Threat | Control |
|---|---|
| Remote network access | The listener is opt-in and binds `127.0.0.1` only. The bind address is not configurable. |
| Local process or web page calling the API | Bearer token on every data route. `corvus serve` refuses to start if the token is unset or shorter than 32 characters. No route is exempt. The desktop app never opens a network listener ([D0022](../decisions/0022-desktop-single-instance-local-socket.md)). |
| Token guessing by timing | Compare SHA-256 digests of presented and expected tokens with `subtle::ConstantTimeEq`. The token type has no `PartialEq`. |
| DNS rebinding | `Host` must be `127.0.0.1`, `localhost` or `[::1]` (with the configured port or none); otherwise `403` before auth. |
| Cross-site requests from a browser | No CORS headers are ever sent, so cross-origin reads and preflights fail; the token is a header, never a cookie. |
| Reaching the unauthenticated inner router | `data_router()` is never bound to a socket; only `serve()` (which wraps it in host + auth layers) is. One function forms the seam and a test asserts every socket route demands the token. |
| Script injection via agent / tracker text | Text-node rendering, sink ban, bundle scan, strict CSP ([05](05-dashboard.md#untrusted-text-p7)). |
| Injected script abusing the desktop app | The page's only data capability is `api`, equivalent to the HTTP API; no shell, filesystem, opener or HTTP-client plugin is exposed; navigation is locked to bundled assets. |
| Prompt injection via tracker text | Tracker text is labelled untrusted wherever copied into tasks/messages; wake adapters cannot forward peer text (there is no placeholder for it); the hook frames messages as peer requests. This reduces, not eliminates, the risk. |
| Token leakage | The token is used only by `corvus serve` and its API clients; it never reaches the dashboard or any URL. Never logged; `Authorization` is redacted from request logs. Never written to disk by Corvus. |
| Secret leakage | Secrets only from environment / env file (mode `0600` enforced). Never in code, logs, the database, exports, or error messages. Config types holding secrets have redacting `Debug` impls. |
| API discovery | No OpenAPI, schema, docs or introspection route. Unknown paths return a bare `404`. |
| SSRF / open redirect via trackers | Tracker host fixed by config; ticket keys validated before use; redirects disabled; timeouts and body caps ([07](07-work-sources.md#outbound-http-rules-both-adapters)). |
| Query injection | SQL always uses bound parameters; no string-built SQL. JQL is built from validated fields with escaping; Linear uses GraphQL variables only. `trusted_schema = OFF`. |
| Path traversal | Agent names are grammar-checked before use in file names (hook, watcher, export). Export `subdir` and artifact paths reject absolute paths, `..` and NUL. |
| Command injection | `git`, the watcher command, the opener and adapter commands are spawned with argument vectors, never via a shell. Adapter files are loaded only if owned by the user and not writable by others. |
| A tool passing another tool's identity | `corvus mcp` takes a conversation id only from the source that belongs to the client named in `initialize` (the environment variable for Claude Code, request metadata for OpenCode); a leaked variable from another tool is ignored. Ids are grammar-checked before they reach SQL or a file name. |
| The model choosing who it is | `conversation` is not in the `register_session` tool schema; the server fills it. A linked conversation always registers as its linked agent. |
| Wake loops / runaway turns | The waiter wakes once per message (shared shown-set, saved before it exits 2; unsaved → no wake), one waiter per process (file lock), never in headless runs, direct mail only. |
| Oversized input | Request body cap 256 KiB; message/task body cap 20,000 chars in the store. |
| Database corruption by a stray tool | WAL + `synchronous = FULL`; one opener sets pragmas; backup before every migration; `corvus admin backup`. |

## Explicit non-goals

- No multi-user authentication or per-agent authorisation. Any holder of the token can act as any agent.
- **No protection against other processes running as the same OS user**: they can open the database file (and
  read the env file) directly. The token guards the network surface, not the file. This is inherent to an
  embedded database and matches the brief's single-machine, single-developer scope.
- No TLS on loopback.
- No encryption of the database at rest (rely on full-disk encryption).

These are recorded as known gaps in [01](01-architecture.md#known-gaps-kept-current-feeds-the-final-architecture-note).
