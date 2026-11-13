# D0020 — The dashboard exists only in the desktop app; there is no browser UI

- **Status:** Accepted (spike pending — see Consequences)
- **Date:** 2026-11-13
- **Supersedes:** [D0018](0018-self-contained-desktop.md), [D0019](0019-dashboard-frontend-two-carriers.md)
- **Plans:** [0000](../plans/0000-roadmap.md), [0006](../plans/0006-dashboard-and-desktop.md), [0011](../plans/0011-distribution.md)
- **Spec:** [system/05](../system/05-dashboard.md) · brief §5 "Dashboard", P6, P7

## Problem

After [D0018](0018-self-contained-desktop.md) and [D0019](0019-dashboard-frontend-two-carriers.md) the dashboard
ran in two hosts: inside the Tauri app over IPC, and in a browser at `/ui` over the HTTP API. The browser host
was kept only because the brief describes it. It carries real weight: a static-shell route, a token-in-fragment
sign-in flow, a paste-token gate, sign-out, a second data carrier, a second CSP, committed build output so the
CLI binary could embed the page, and an optional listener inside the desktop app. Owner direction (2026-11-13):
**"Just the app, no browser."** What is the dashboard's delivery model now, and what happens to the pieces that
existed for the browser?

## Options

1. **Keep both hosts** (D0018 + D0019 as they were).
2. **App only; keep the HTTP API** as an opt-in programmatic interface served by `corvus serve`.
3. **App only; drop the HTTP API too**, leaving CLI, MCP and the app.

## Trade-offs

### Keep both hosts
- **For:** matches the brief's dashboard section literally; a dashboard is available on a headless machine.
- **Against:** two auth models and two carriers to build, test and secure for a second way of seeing the same
  screen; the owner does not want it.

### App only, HTTP API kept
- **For:** removes the whole browser sign-in surface and the token from the UI; the app never opens a socket,
  so it has no network surface in any configuration; the CLI binary no longer contains or needs the frontend, so
  building it needs no Node and nothing has to be committed as build output; the brief's HTTP API (a separate
  requirement from the dashboard) still exists for scripts and other tools.
- **Against:** no dashboard without the desktop app (headless users have the CLI and the API); the brief's
  `/ui` items — fragment token, gate form, sign-out, "shell served without auth" — are deliberately not built;
  end-to-end UI tests can no longer simply point a browser at a URL.

### App only, no HTTP API
- **For:** smallest surface of all.
- **Against:** removes a brief interface nobody asked to remove; scripts and non-MCP tools would lose their
  integration point; the app's IPC route table would have no second consumer keeping it honest.

## Decision

**Option 2.** The dashboard is the desktop app, and only the desktop app.

- Carried forward from D0018, unchanged: the app is self-contained — it links `corvus-core`, opens the database
  itself, and its frontend reaches the service layer through one IPC command (`api`) that calls the HTTP route
  table (`data_router()`) in memory. Close hides to tray; the app is a background-leader candidate; it bundles
  the `corvus` binary. The reasoning for IPC-into-the-router over typed commands or a loopback server is in D0018.
- Carried forward from D0019, unchanged: Svelte 5 with runes, TypeScript, plain Vite, `fragments: 'tree'`, and
  the layered text-safety controls. The reasoning for Svelte over Rust/WASM, vanilla TypeScript or a native GUI
  is in D0019.
- **Removed:** the `/ui` route and static shell in `corvus-http`; the HTTP carrier, token handling, gate form
  and sign-out in the frontend; the optional HTTP listener in the desktop app; committed `ui/dist`.
- **Kept, narrowed:** the HTTP API is served only by `corvus serve`, only when a token is configured, and is
  purely programmatic. Every route on it requires the token — there is no unauthenticated route left.
- `corvus ui` launches the desktop app.

## Consequences

- The frontend is built by the Tauri build and embedded in the app only. Building `corvus` needs Rust alone;
  building the app needs Node (a developer and release concern, never a user's).
- **Spike first** ([plan 0006](../plans/0006-dashboard-and-desktop.md) step 1): confirm a `fragments: 'tree'`
  build has no HTML sinks and runs in the Tauri webview under the app's CSP. If it cannot be made sink-free,
  supersede this record's frontend choice with vanilla TypeScript before building panels.
- UI tests: unit tests with a fake carrier; browser-engine tests load the built bundle with a test-only shim
  for the IPC call; the real app is exercised by WebDriver on Linux where available and by a manual checklist
  on macOS ([system/09](../system/09-testing.md)).
- Departures from the brief's dashboard wording are listed in
  [system/11](../system/11-defaults.md#deliberate-departures-from-the-briefs-wording).
- Revisit if a headless or remote dashboard is ever wanted; D0019's two-carrier design is the starting point.

## Discussion

- 2026-11-13 — Created after the owner chose a desktop-only dashboard. Supersedes D0018 and D0019, carrying
  their architecture and framework choices forward and removing the browser host. Interpretation to confirm:
  the HTTP API was kept as an opt-in programmatic interface, since the direction was about the UI.
  The `fragments: 'tree'` option is from memory of the Svelte 5 compiler options and has not been exercised.
