# 05 — Dashboard and desktop app

The dashboard is the **Tauri 2 desktop app**, and only that: there is no browser UI and no `/ui` route
([D0020](../decisions/0020-desktop-only-dashboard.md)). The app is self-contained — double-click and Corvus
works, with no server, token, port or CLI install. Panels and behaviour are brief §5 "Dashboard", minus the
browser sign-in items; this file fixes the build, the security model and the app.

## How it works

```
 Corvus desktop app — one process
 ┌───────────────────────────────────────────────────────────────┐
 │  webview:  Svelte dashboard (bundled assets only)             │
 │      │  invoke('api', {method, path, body})      ← Tauri IPC  │
 │      ▼                                                        │
 │  Rust:  api command ──► data_router()  (the HTTP route table, │
 │                              │           called in memory)    │
 │                              ▼                                │
 │                         corvus-core services ──► SQLite file  │
 │  background leader: housekeeping, Linear/Jira polling         │
 └───────────────────────────────────────────────────────────────┘
        no listening socket, in any configuration
```

Agents never talk to the app. They write to the same SQLite file through `corvus mcp`; the dashboard polls it.

## Frontend (`ui/`)

- Svelte 5 with runes, TypeScript, Vite. Plain Svelte, **no SvelteKit** — one page, no router, no SSR.
- Compiler option `fragments: 'tree'` so templates are built with DOM calls instead of `innerHTML`
  (verified by the bundle scan below; see the spike in [plan 0006](../plans/0006-dashboard-and-desktop.md)).
- Built by the Tauri build and embedded in the app binary. No inline `<script>` or `<style>`, no external
  origins, no web fonts from a CDN. `ui/dist` is a build product and is not committed.
- The `corvus` CLI binary does not contain the frontend; building it needs no Node.
- TypeScript API types are generated from the Rust request/response structs (`ts-rs`) into `ui/src/lib/types/`
  and committed; CI fails on drift.
- The bundle contains no data: everything shown is fetched at run time.

### Structure

```
ui/src/
  main.ts            mount
  App.svelte         dashboard
  lib/api.ts         THE data layer: request(method, path, body) → invoke('api', …); maps errors
  lib/poll.svelte.ts 3 s / 30 s polling with pause and visibility handling
  lib/sourceRules.ts parse/format "AA, LAI: To Do, In Progress" (pure, unit-tested)
  lib/types/         generated from Rust
  panels/            Header, Sessions, Activity, Decisions, Messages, Tasks, Sources
```

`api.ts` is the only module that touches IPC. Panels call `api.request('GET', '/api/overview')` and nothing
else, with the same paths, bodies and error shapes as the HTTP API ([04](04-interfaces.md#http-api)) — so the
route tests cover what the dashboard uses, and tests can substitute a fake `api`.

There is no sign-in, token, gate form or sign-out: the page is the app's own bundled content and the user is
whoever launched the app.

### Panels

As brief §5: header chips (sessions by presence, open/claimed tasks, unread total with click-to-filter, expired
leases, blocked-by-cancelled); agents & sessions (presence, client, last seen, unread, current task + lease
time, toggle for offline/ended); activity feed; decisions; messages (newest first, thread, read/unread-by,
filters, compose as the operator to one agent or everyone, reply with `in_reply_to` defaulting to `answer` for
a question); tasks board (Open / Claimed / Done / Cancelled with priority, blocked flags, holder, cancel);
pause and theme toggle.

Agents & sessions also shows, per agent, its tool and conversation directory and a **not reachable** flag when
it has unread direct mail and nothing can deliver it ([06](06-delivery-helpers.md#waiter-corvus-hook-wait),
plan 0012).

### Sources panel

The brief's "Jira panel" is generalised: one **Sources** panel with a tab per enabled source (Linear, Jira),
hidden entirely when none is enabled. Each tab is the brief's panel unchanged — status columns plus "Done in
last N days", cards, Add to queue / Queue now / Remove from queue, Sync now, the Filter form with rules written
one per line as `AA, LAI: To Do, In Progress` (`*` = any project), clickable status suggestions, last-sync time
or error. For Linear the form labels "projects" as "teams"; the syntax is identical.

### Data flow

- `GET /api/overview` + `GET /api/messages` every ~3 s; `GET /api/sources/{source}` every ~30 s for each
  enabled source. Polling pauses on the pause button and while the window is hidden. Requests never overlap
  (skip a tick if one is in flight).
- Operator actions: compose / reply → `POST /api/messages` or `/api/messages/direct`; cancel task →
  `POST /api/tasks/{id}/cancel`; source actions per [07](07-work-sources.md#http-surface). Each sends an
  `idempotency_key` generated per click.
- **Viewing never acks.** The dashboard has no code path that calls the ack route.

### Untrusted text (P7)

All agent- and tracker-authored strings are rendered through Svelte text interpolation (`{value}`), which
creates text nodes. Defence in depth:

| Layer | Control |
|---|---|
| Source | Lint bans `{@html}`, `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval`, `new Function`, `style=` attributes. |
| Build | A test scans the built bundle for the same sinks and fails on any hit. |
| Engine | The app's CSP (below). |
| Links | A ticket link is rendered only if `new URL(u).protocol === 'https:'`; clicking it never navigates the webview — Rust hands it to the system browser. Otherwise the key is plain text. |
| Behaviour | A test posts a message whose body is `<img src=x onerror=…>` and asserts it appears as literal text. |
| Blast radius | Even if script ran in the page, its only capability is `api`, which can do what the operator can already do in the UI — no shell, filesystem or network access. |

### Content security policy

Set in the Tauri configuration:

```
default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self';
connect-src <only what Tauri IPC requires>; base-uri 'none'; form-action 'none'; frame-ancestors 'none';
require-trusted-types-for 'script'      (kept if the webview enforces it and Tauri's IPC tolerates it)
```

**Spike result (2026-11-13, macOS 26 / WKWebView, Tauri 2.11, Svelte 5.57 with `fragments: 'tree'`):**
`connect-src ipc: http://ipc.localhost` (the `ipc:` scheme on macOS; Tauri falls back to `http://ipc.localhost` on
other platforms). Trusted Types stays: WKWebView enforces `require-trusted-types-for 'script'` (a raw HTML
assignment throws), and Tauri's IPC works under it. The built bundle contains no HTML sinks; its only absolute
URLs are `https://svelte.dev/e/…` links inside Svelte's runtime warning messages, which the scan allows by
exact prefix (they are text, never loaded). Linux / WebKitGTK was not checked (no Linux machine in the build
environment).

### Layout

Light and dark themes via CSS custom properties (`prefers-color-scheme` default, toggle overrides). Panels are
a CSS grid that collapses to one column under ~720 px window width; the task and source boards scroll
horizontally when narrow.

## Desktop app (`crates/corvus-desktop`, Tauri 2)

- **In-process core.** The app links `corvus-core`, opens (and if needed creates and migrates) the database,
  and is a normal Corvus process ([01](01-architecture.md#processes)).
- **UI over IPC.** One Tauri command, `api(method, path, body) → {status, body}`, builds a request in memory and
  calls `data_router()` from `corvus-http` with `tower::ServiceExt::oneshot`. No socket is involved.
- **No network listener, ever.** The app never binds a TCP or UDP port, whether or not a token is configured.
  The HTTP API is served only by `corvus serve` ([04](04-interfaces.md#http-api)). The single-instance guard's
  local rendezvous (below) is the one exception, and it carries no data ([D0022](../decisions/0022-desktop-single-instance-local-socket.md)).
- **Background work.** While running, the app competes for the background-leader lock and, when it holds it,
  runs housekeeping and tracker polling ([10](10-config-ops.md#background-leader)).
- **Window and tray.** Closing the window hides it; the app stays in the tray so background work continues.
  Tray menu: Open, Pause/Resume background work, Quit. After Quit, agents keep working against the database.
  On macOS the tray icon is a monochrome template image in the menu bar, and while the window is hidden the app
  leaves the Dock and the app switcher (it returns on Open or relaunch).
- **Connect an agent.** A help view lists the available adapters (and whether each tool is installed) and shows the MCP and hook configuration snippets with the real path of the
  bundled `corvus` binary, and an "Install command-line tool" action that links it into the user's `PATH`
  (`~/.local/bin`; never needs elevated rights).
- **Single instance.** A second launch focuses the existing window. `corvus ui` does the same from a terminal
  ([04](04-interfaces.md#cli-corvus)). The rendezvous is a Unix-domain socket in `/tmp` on macOS and a D-Bus
  session name on Linux; the callback ignores the caller's arguments and only shows the window
  ([D0022](../decisions/0022-desktop-single-instance-local-socket.md)).
- **Isolation**:
  - IPC capabilities are limited to `api` plus a few app commands (`app_info`, `install_cli`,
    `set_background_paused`). No shell, filesystem, HTTP-client or opener plugin is exposed to the page.
  - The webview loads only the bundled assets. A navigation guard cancels every other navigation and every
    new-window request; `https:` targets are handed to the system browser from Rust.
  - Tracker credentials are read from the environment / env file and never reach the webview.
- **Development**: `tauri dev` runs the Vite dev server for hot reload. That server is a development tool only;
  nothing in a release build listens on a port.
