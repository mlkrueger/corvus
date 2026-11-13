# D0018 — The Tauri 2 app is self-contained: core in-process, UI over IPC

- **Status:** Superseded by [D0020](0020-desktop-only-dashboard.md)
- **Date:** 2026-11-13
- **Supersedes:** [D0008](0008-tauri-shell-role.md)
- **Plans:** [0000](../plans/0000-roadmap.md), [0006](../plans/0006-dashboard-and-desktop.md), [0011](../plans/0011-distribution.md)
- **Spec:** [system/05](../system/05-dashboard.md#desktop-app-cratescorvus-desktop-tauri-2) · brief §5 "Dashboard", P6, P7

## Problem

Owner direction (2026-11-13): **the desktop app is self-contained.** [D0008](0008-tauri-shell-role.md) made
Tauri a thin window around a page served by a separately running server; that is no longer acceptable.
Double-clicking the app must give a working Corvus with nothing else installed or started. The brief still
requires an HTTP API and a dashboard page at `/ui`. How does the app get data, what does it run, and how do the
two dashboards relate?

## Options

1. **In-process core, typed Tauri commands.** The app links `corvus-core`; the frontend calls one Tauri command
   per operation.
2. **In-process core, one generic IPC command into the HTTP router.** The app links `corvus-core` and
   `corvus-http`; the frontend sends `(method, path, body)` over IPC and the app dispatches it into the same
   axum router in memory — no socket.
3. **In-process loopback server.** The app starts the HTTP server on a port and its webview uses it over HTTP.
4. **Sidecar daemon.** The app spawns `corvus serve` as a child and talks to it.

## Trade-offs

### Typed Tauri commands
- **For:** idiomatic Tauri; typed end to end.
- **Against:** a fifth transport — every route duplicated as a command, and the frontend needs two data layers
  (commands in the app, fetch in the browser dashboard) that drift apart.

### Generic IPC command into the router
- **For:** one route table, one set of handlers, one frontend data layer with two carriers (`invoke` in the
  app, `fetch` in a browser); no port, no token, no listener — the app has zero network surface by default;
  every route test covers both dashboards.
- **Against:** an unusual pattern that needs a clear seam (the IPC path enters *below* the token and host
  checks, which only make sense for sockets); requests are JSON-encoded twice; the page that renders untrusted
  text holds one IPC capability, so that command must expose nothing beyond what the HTTP API already does.

### In-process loopback server
- **For:** the frontend is identical in app and browser.
- **Against:** needs a token before the UI can work — but the brief forbids inventing or storing secrets, so a
  zero-config app could not start; always opens a port.

### Sidecar daemon
- **For:** process isolation.
- **Against:** all the drawbacks of a required daemon ([D0015](0015-process-topology-single-binary.md)) plus
  child supervision.

## Decision

**Option 2.** The app embeds the core and opens the database like any other Corvus process. Its bundled Svelte
frontend talks to the service layer through a single `api` IPC command that calls the axum router in memory.
While running, the app is a candidate background leader (housekeeping, tracker polling). If — and only if —
`CORVUS_API_TOKEN` is configured, it also starts the real loopback HTTP listener for browsers and other
clients. Closing the window hides to the tray; Quit stops the app, and agents keep working against the file.

## Consequences

- The dashboard exists in two hosts from one bundle: inside the app (IPC) and at `/ui` from `corvus serve` or
  the app's optional listener (HTTP + token). [D0019](0019-dashboard-frontend-two-carriers.md).
- IPC capabilities: `api` and a few app-level commands (version, paths, install CLI). Nothing else — no shell,
  no filesystem, no arbitrary HTTP plugin.
- The webview may only load the bundled assets; all other navigation and new windows go to the system browser
  (https only).
- The app bundles the `corvus` binary so agents can be connected without a separate download
  ([plan 0011](../plans/0011-distribution.md)).
- The brief's "token on every data route" applies to the socket surface. IPC is not a network route; the seam
  is tested so that no socket request can reach the unauthenticated inner router.

## Discussion

- 2026-11-13 — Created after the owner chose a self-contained app. Supersedes D0008.
- 2026-11-13 — The owner chose a desktop-only dashboard. The in-process core and IPC-into-the-router design carry forward; the optional HTTP listener in the app and the second (browser) host are removed. Superseded by D0020.
