# Plan 0004 — Transports: CLI, MCP server, HTTP API

- **Status:** Done
- **Depends on:** [0003](0003-muninn-hermod.md)
- **Spec:** [system/04](../system/04-interfaces.md) · [system/08](../system/08-security.md) · brief §5 (P6)

## Goal

Every service operation is reachable through the `corvus` CLI, the MCP stdio server and the HTTP API, with no
business logic in any handler. Agents need only `corvus mcp`; the HTTP API is opt-in and fully secured.

## Scope

**In:** the async `Corvus` facade; all CLI subcommands except `hook`/`watch`/`codex`/`up`/`down`/`status`
(0007) and source sync (0008/0009); `corvus-mcp` with all twenty tools; `corvus-http` with the inner
`data_router()` and the outer secured `serve()`; `corvus serve`.

**Out:** the desktop app, its IPC command and `corvus ui` (0006); background loops (0005); source routes (0008).
There is no `/ui` route — the HTTP API serves no pages.

## Work breakdown

- [x] 1. Shared request/response structs in `corvus-core::api` (serde + JSON-schema derive), one per operation.
- [x] 2. `Corvus` facade: async methods that run the synchronous services on the blocking pool.
- [x] 3. CLI subcommands, JSON output, exit codes; `corvus tell`.
- [x] 4. `corvus-mcp`: tools, instructions = `GUIDANCE`, remembered session, end-on-EOF.
- [x] 5. `corvus-http::data_router()`: routes per the table in system/04, each a one-call handler; long-poll
      route for inbox wait; error mapping including `Busy` → 503.
- [x] 6. `corvus-http::serve()`: host allow-list → auth → body limit → timeout → logging with redaction;
      security headers; bare 404. No static or page routes.
- [x] 7. `corvus serve`: token check at startup, loopback bind, graceful shutdown.
- [x] 8. Architecture checks: `corvus-http`, `corvus-mcp`, `corvus-cli` do not depend on `rusqlite`; handlers
      contain a single service call (review checklist).

## Verification

- [x] CLI: `assert_cmd` tests for each command group against a temp database, JSON validity, exit codes;
      `tell` uses the operator name; commands work with an empty environment.
- [x] MCP: spawn `corvus mcp` **with no token and no server**, initialize, list tools (exactly the twenty), run
      register → create → claim → complete → post → inbox → ack → end; instructions equal `GUIDANCE`.
- [x] Two `corvus mcp` processes on one database: one waits, the other posts, the waiter wakes.
- [x] **Auth on every data route:** enumerate `serve()`'s routes programmatically; each returns 401 without a
      token and with a wrong token; **no route is exempt**; `/ui` is a 404.
- [x] **Seam:** `data_router()` is not `pub` outside the workspace's desktop/http crates and no code path binds
      it to a listener (test + grep in lint).
- [x] Token comparison goes through the constant-time function; the token is a newtype without `PartialEq`.
- [x] Host allow-list: `evil.example`, `127.0.0.1.evil.example`, missing Host → 403; loopback names pass.
- [x] No CORS headers on any response; `OPTIONS` preflight gets no allow headers.
- [x] `serve` refuses to start without a token or with a short token, and binds only `127.0.0.1`.
- [x] Error mapping: one test per `CorvusError` variant → 404 / 409 / 422 / 503 / 500 with the JSON shape;
      malformed JSON and bad path params → 422; internal errors leak no detail.
- [x] No docs/schema route: `/docs`, `/openapi.json`, `/api`, `/schema` → 404.
- [x] Round trip per route against a temp database (create → list → get, claim → renew → complete, post → inbox → ack …).
- [x] Long poll returns on a message and at timeout; survives client disconnect without leaking the waiter.
- [x] Logs: a request with a token and a message body produces log lines containing neither.
- [ ] By hand: connect Claude Code with `corvus mcp` as the only configuration and run register → claim →
      complete. (Not done — see *Unverified*.)
- [x] By hand: `curl` each route group against `corvus serve`.

## Exit criteria

- [x] `make lint` and `make test` pass.
- [x] An agent can be connected with a config containing only the command `corvus mcp` — no token, URL or path
      to a database. (Verified with a scripted MCP client against the real binary under an empty environment;
      not yet with Claude Code itself.)

## Open questions

- None. (The `rmcp` 3.10 API was confirmed against its source; notes are in
  [D0014](../decisions/0014-rust-stack-rusqlite.md)'s Discussion.)

## Unverified / deferred

- **A real Claude Code client** was not connected. `corvus mcp` was exercised with a scripted JSON-RPC client
  over the real binary's stdin/stdout, with an empty environment (no token, URL or database path), including two
  MCP processes waking each other. Connecting Claude Code itself needs the owner's machine and client.
- `wait_for_inbox` interrupted by stdin EOF mid-wait is not tested.
- `corvus serve` was exercised by hand with `curl` (one call per route group, auth/host/404 checks, SIGTERM exit 0,
  no token in logs) on this machine.

## Notes from implementation

- `corvus-http`: `route_table()` and `data_router()` are generated from one list, so the auth test enumerates
  every route. Layers, outermost first: security headers → request log → host allow-list → bearer auth →
  Content-Length check → 256 KiB body limit → 130 s timeout → router. Bodies over the cap get a JSON 413.
  On shutdown, in-flight requests get 5 s (long polls do not hold shutdown for 120 s).
- `corvus-mcp`: tool input schemas are `schemars::schema_for!` of the `corvus-core::api` request structs; the
  remembered agent and session become optional, and HTTP path ids become properties. Arguments are filled at the
  JSON level and deserialised into the core struct, so no request shape is defined twice.
- `make lint` gained the `data_router` seam check.
- Also fixed here: plan 0003 was committed with `cargo fmt --check` failing (and so clippy unrun); its formatting
  and one `type_complexity` lint in `hermod.rs` are corrected in this plan's commit.
- Choices where the spec was silent: [system/11](../system/11-defaults.md) rows 47–52.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0014 | Synchronous services over rusqlite, with an async facade for the tokio transports; axum for HTTP, rmcp for MCP, clap for the CLI. | [0014](../decisions/0014-rust-stack-rusqlite.md) |
| D0015 | CLI and MCP open the database directly and need no server or token; the HTTP API exists only when a token is configured. | [0015](../decisions/0015-process-topology-single-binary.md) |
