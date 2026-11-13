# Plan 0006 — Dashboard and self-contained desktop app (Svelte + Tauri 2)

- **Status:** Ready
- **Depends on:** [0004](0004-transports.md), [0005](0005-housekeeping.md)
- **Spec:** [system/05](../system/05-dashboard.md) · [system/08](../system/08-security.md) · brief §5 "Dashboard" (P6, P7)

## Goal

A self-contained Tauri 2 desktop app that is the Corvus dashboard: launch it with nothing else running and see
and operate live state. All untrusted text renders as text. There is no browser UI.

## Scope

**In:** the `ui/` Svelte app and its build; generated TypeScript types; the IPC data layer; all panels except
Sources; `corvus-desktop`: in-process core, `api` command into `data_router()`, background leadership, tray,
single instance, navigation guard, "Connect an agent" view; `corvus ui`.

**Out:** the Sources panel ([0008](0008-work-sources-and-linear.md)); bundling the CLI binary, signing and
release artifacts ([0011](0011-distribution.md)); any browser-hosted dashboard, token sign-in or `/ui` route
(not built — [D0020](../decisions/0020-desktop-only-dashboard.md)); native notifications and tray badges (future).

## Work breakdown

- [ ] 1. **Spike (gate for everything below):** a minimal Tauri 2 app with a one-component Svelte 5 page built
      with `fragments: 'tree'`, calling one IPC command. (a) The bundle sink scan passes. (b) It runs under the
      strict CSP on macOS and Linux; record the exact `connect-src` Tauri needs and whether
      `require-trusted-types-for` can stay. Fail on (a) → supersede the frontend choice in
      [D0020](../decisions/0020-desktop-only-dashboard.md) (vanilla TypeScript) before step 2.
- [ ] 2. `ui/` scaffold: Vite + TS + Svelte 5, lint rules banning sinks, `ts-rs` type generation with a drift
      check, bundle-scan test, wired into the Tauri build.
- [ ] 3. `corvus-desktop` core: open/migrate the database on start; the `api` command dispatching into
      `data_router()`; capabilities limited to the declared commands; CSP in the Tauri config.
- [ ] 4. `api.ts` (the only IPC caller), error mapping, polling with pause and visibility handling, theme toggle.
- [ ] 5. Header chips; Agents & sessions panel (presence, unread, current task, lease countdown, offline/ended toggle).
- [ ] 6. Activity feed; Decisions list.
- [ ] 7. Messages panel: list, filters (all / unread / by agent, chip click-through), compose, reply
      (`in_reply_to`, `answer` default for questions).
- [ ] 8. Tasks board: four columns, priority, blocked flags, holder, cancel action.
- [ ] 9. Responsive layout and both themes.
- [ ] 10. App behaviour: background-leader participation; close-to-tray, tray menu, single instance;
      navigation guard; ticket links to the system browser.
- [ ] 11. "Connect an agent" view (config snippets); `corvus ui` launches or focuses the app.

## Verification

- [ ] **No sinks:** scan of the built bundle finds no `innerHTML`, `outerHTML`, `insertAdjacentHTML`,
      `document.write`, `eval(`, `new Function`, no inline `<script>`/`<style>`, no `style=` attributes, no
      external URLs.
- [ ] **Bundle has no data:** the built assets contain no agent names, titles, bodies or secrets from a seeded
      database (they are fetched at run time only).
- [ ] Generated TypeScript types match the Rust API structs (drift check).
- [ ] Unit (vitest, fake `api`): rule-line parse/format round trip; message filter logic; https-only link
      guard; polling pause and no-overlap; error mapping; each panel renders seeded data.
- [ ] Bundle in a browser engine (Playwright WebKit and Chromium, loading the built assets from a test-only
      static server, with a **test-only shim** that implements the `api` IPC call against a throwaway
      `corvus serve`; the shim lives in the test harness, not in the shipped bundle):
  - [ ] **Hostile text:** message body, task title, agent metadata, decision text and ticket summary containing
        `<img src=x onerror=…>` / `<script>` render literally; no script runs; no CSP violation is reported.
  - [ ] compose to one agent and to everyone; reply defaults to `answer` for a question; **viewing creates no receipts**.
  - [ ] cancel an open task from the board; blocked-by-cancelled flag appears on its dependent.
  - [ ] pause stops requests; narrow window and dark theme render without overlap (screenshots reviewed).
- [ ] App, automated without a window:
  - [ ] the `api` command returns the same status and body as the HTTP route for a sample of every route group,
        including error shapes.
  - [ ] **the app process opens no listening socket**, with and without `CORVUS_API_TOKEN` set.
  - [ ] navigation guard allows only bundled assets; rejects every external and loopback URL.
  - [ ] capability config exposes only `api`, `app_info`, `install_cli`, `set_background_paused`.
- [ ] App, automated with a window on Linux (WebDriver via `tauri-driver`, if it proves workable — see open
      questions): launch, see seeded data, post a message, hostile text renders literally.
- [ ] App, by hand on macOS (recorded in this plan with date): on a machine with nothing else running, launch
      the app → dashboard shows live data; run `corvus tell` from a terminal → message appears; hostile text
      renders literally; click a ticket link → system browser; close window → still in tray and still leader;
      Quit → `corvus mcp` still works.

## Exit criteria

- [ ] `make lint`, `make test`, `make test-e2e` pass.
- [ ] The manual macOS checklist is filled in below.
- [ ] `corvus-http` has no `/ui` route and no static assets; the `corvus` binary builds without Node.

## Open questions

- Do the Tauri webviews (WKWebView on macOS, WebKitGTK on Linux) enforce `require-trusted-types-for`, and does
  Tauri's IPC bootstrap coexist with it? Determined in step 1. If not, the directive is dropped and the bundle
  scan, the hostile-text test and the single-capability design carry the guarantee.
- Is `tauri-driver` workable for automated window tests on Linux? It does not support macOS. If it is not
  workable, window-level behaviour stays on the manual checklist and that is reported as such.
- Current Tauri 2 API names for navigation / new-window handling, single-instance and tray — confirm against
  Tauri docs at step 3.
- Package manager for `ui/` (npm vs pnpm) — pick at step 2, add to [defaults](../system/11-defaults.md).

## Unverified / deferred

—

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0020 | The dashboard exists only in the self-contained Tauri app: Svelte 5 frontend, one IPC command into the HTTP route table in memory, no browser UI, no `/ui` route, and the app never opens a listener. A spike confirms the bundle is sink-free, with vanilla TypeScript as the fallback. | [0020](../decisions/0020-desktop-only-dashboard.md) |
