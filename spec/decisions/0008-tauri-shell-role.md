# D0008 — Tauri 2 is a native shell around the served dashboard

- **Status:** Superseded by [D0018](0018-self-contained-desktop.md)
- **Date:** 2026-11-13
- **Plans:** [0000](../plans/0000-roadmap.md), [0006](../plans/0006-dashboard-and-desktop.md)
- **Spec:** [system/05](../system/05-dashboard.md) · brief §5 "Dashboard", P6, P7

## Problem

The owner wants Tauri 2 for the UI. The brief specifies the dashboard as a single static page served at `/ui`,
authenticated by a token in the URL fragment, fetching everything from the authenticated HTTP API, and `corvus
ui` "opens the dashboard signed in". How do the two fit together — what does the Tauri app contain, how does it
get data, and who owns the server process?

## Options

1. **Shell around the served page.** The Tauri window navigates to `http://127.0.0.1:<port>/ui#token=…`. One
   frontend, served by the server; Tauri adds a native window, tray and stack control.
2. **Bundled frontend + HTTP.** The frontend ships inside the Tauri app (`tauri://` origin) and calls the
   loopback API cross-origin.
3. **Bundled frontend + IPC.** The frontend calls Tauri commands, which call the service layer in-process —
   a fifth transport; the app may embed the server too.
4. **Tauri only.** Drop `/ui`; the dashboard exists only in the desktop app.

## Trade-offs

### Shell around the served page
- **For:** exactly one dashboard and one auth path, identical in browser and app; the brief's `/ui` contract
  and its tests hold unchanged; the dashboard origin gets no Tauri capabilities, so untrusted text cannot reach
  native APIs even if rendering were compromised; smallest amount of Tauri-specific code.
- **Against:** the app is useless until the server runs (mitigated by a bundled "not running → Start" page);
  loading a non-bundled origin is not the Tauri default and needs a strict navigation guard; no offline UI.

### Bundled frontend + HTTP
- **For:** UI loads instantly even with the server down; conventional Tauri layout.
- **Against:** the API must allow a cross-origin caller (CORS for the Tauri origin), weakening the
  "no CORS at all" posture; two ways to serve the same frontend to keep in sync and test.

### Bundled frontend + IPC
- **For:** no HTTP or token for the desktop UI; could run with no server.
- **Against:** every route duplicated as a Tauri command; the frontend needs two data layers; the GUI process
  needs the database URL; puts untrusted-text rendering in a context that has native capabilities.

### Tauri only
- **For:** one host.
- **Against:** drops a brief requirement (`/ui` in a browser) and its security tests.

## Decision

**Option 1.** Tauri 2 hosts the same page the server serves. The app's own bundled content is one small local
page for the not-running state; only that page has IPC capabilities (`start_stack`, `stop_stack`, `probe`). The
server stays a detached daemon started by the same `scripts/up.sh` as `make up`, so closing the window never
stops coordination for the agents.

## Consequences

- Navigation is restricted to the bundled page and the loopback `/ui`; ticket links open in the system browser.
- The shell reads the token from the env file because GUI apps do not inherit the shell environment.
- Native extras (tray unread badge, notifications) can be added later inside the shell without touching the
  dashboard; they are out of scope for v1.
- Revisit if an offline-capable or server-less desktop app becomes a goal (that leads to option 3 and a
  re-think of [D0001](0001-storage-postgresql.md)).

## Discussion

- 2026-11-13 — Initial record. Flagged to the owner: this is the narrowest reading of "Tauri for UI". If the
  intent was a fully self-contained desktop app, option 3 is the alternative.
- 2026-11-13 — The owner chose a self-contained desktop app (option 3 here, refined). Superseded by D0018.
