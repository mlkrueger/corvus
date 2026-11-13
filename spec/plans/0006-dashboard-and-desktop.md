# Plan 0006 — Dashboard and self-contained desktop app (Svelte + Tauri 2)

- **Status:** In progress
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

- [x] 1. **Spike (gate for everything below):** a minimal Tauri 2 app with a one-component Svelte 5 page built
      with `fragments: 'tree'`, calling one IPC command. (a) The bundle sink scan passes. (b) It runs under the
      strict CSP on macOS and Linux; record the exact `connect-src` Tauri needs and whether
      `require-trusted-types-for` can stay. Fail on (a) → supersede the frontend choice in
      [D0020](../decisions/0020-desktop-only-dashboard.md) (vanilla TypeScript) before step 2.
- [x] 2. `ui/` scaffold: Vite + TS + Svelte 5, lint rules banning sinks, `ts-rs` type generation with a drift
      check, bundle-scan test, wired into the Tauri build.
- [x] 3. `corvus-desktop` core: open/migrate the database on start; the `api` command dispatching into
      `data_router()`; capabilities limited to the declared commands; CSP in the Tauri config.
- [x] 4. `api.ts` (the only IPC caller), error mapping, polling with pause and visibility handling, theme toggle.
- [x] 5. Header chips; Agents & sessions panel (presence, unread, current task, lease countdown, offline/ended toggle).
- [x] 6. Activity feed; Decisions list.
- [x] 7. Messages panel: list, filters (all / unread / by agent, chip click-through), compose, reply
      (`in_reply_to`, `answer` default for questions).
- [x] 8. Tasks board: four columns, priority, blocked flags, holder, cancel action.
- [x] 9. Responsive layout and both themes.
- [x] 10. App behaviour: background-leader participation; close-to-tray, tray menu, single instance;
      navigation guard; ticket links to the system browser.
- [x] 11. "Connect an agent" view (config snippets); `corvus ui` launches or focuses the app.

## Verification

- [x] **No sinks:** scan of the built bundle finds no `innerHTML`, `outerHTML`, `insertAdjacentHTML`,
      `document.write`, `eval(`, `new Function`, no inline `<script>`/`<style>`, no `style=` attributes, no
      external URLs.
- [x] **Bundle has no data:** the built assets contain no agent names, titles, bodies or secrets from a seeded
      database (they are fetched at run time only).
- [x] Generated TypeScript types match the Rust API structs (drift check).
- [x] Unit (vitest, fake `api`): rule-line parse/format round trip; message filter logic; https-only link
      guard; polling pause and no-overlap; error mapping; each panel renders seeded data.
- [x] Bundle in a browser engine (Playwright WebKit and Chromium, loading the built assets from a test-only
      static server, with a **test-only shim** that implements the `api` IPC call against a throwaway
      `corvus serve`; the shim lives in the test harness, not in the shipped bundle):
  - [x] **Hostile text:** message body, task title, agent metadata, decision text and ticket summary containing
        `<img src=x onerror=…>` / `<script>` render literally; no script runs; no CSP violation is reported.
  - [x] compose to one agent and to everyone; reply defaults to `answer` for a question; **viewing creates no receipts**.
  - [x] cancel an open task from the board; blocked-by-cancelled flag appears on its dependent.
  - [x] pause stops requests; narrow window and dark theme render without overlap (screenshots reviewed).
- [x] App, automated without a window:
  - [x] the `api` command returns the same status and body as the HTTP route for a sample of every route group,
        including error shapes.
  - [x] **the app process opens no listening socket**, with and without `CORVUS_API_TOKEN` set.
  - [x] navigation guard allows only bundled assets; rejects every external and loopback URL.
  - [x] capability config exposes only `api`, `app_info`, `install_cli`, `set_background_paused`.
- [ ] App, automated with a window on Linux (WebDriver via `tauri-driver`, if it proves workable — see open
      questions): launch, see seeded data, post a message, hostile text renders literally.
- [ ] App, by hand on macOS (recorded in this plan with date): on a machine with nothing else running, launch
      the app → dashboard shows live data; run `corvus tell` from a terminal → message appears; hostile text
      renders literally; click a ticket link → system browser; close window → still in tray and still leader;
      Quit → `corvus mcp` still works.

## Exit criteria

- [x] `make lint`, `make test`, `make test-e2e` pass.
- [ ] The manual macOS checklist is filled in below.
- [x] `corvus-http` has no `/ui` route and no static assets; the `corvus` binary builds without Node.

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

- **Manual macOS checklist — not yet done (needs the owner at the machine).** The app was launched against a
  temporary state directory with hostile seed data and a live `corvus tell`; it ran with no warnings and no
  listening TCP/UDP socket, but its window could not be looked at from the build session (the terminal has no
  Screen Recording permission). To do, and record here with a date: launch with nothing else running → live
  data; `corvus tell` from a terminal → message appears; hostile text renders literally; close window → still in
  the tray and still the leader (`corvus status` / `run/leader.json`); tray Pause/Resume; Quit → `corvus mcp`
  still works.
- **Linux / WebKitGTK** was not checked at all (no Linux machine): neither the spike's CSP/Trusted-Types result
  nor `tauri-driver` window tests. The Linux row stays open.
- **Single instance uses a Unix-domain socket** (`tauri-plugin-single-instance` rendezvous at
  `/tmp/<identifier>_si.sock`). It is not a network port, so the "no listening TCP/UDP socket" test passes, but
  it is a local listening socket, which the spec's "never opens a socket" does not allow for. Recorded as a known
  gap pending an owner decision (keep it, or replace the plugin with a lock file and lose "second launch focuses
  the window" on Linux).
- The tray icon is the full-colour app icon, not a macOS template image.
- The e2e "bundle in a browser engine" tests use a shim that proxies to `corvus serve`; the IPC shape is also
  checked against the real Rust `dispatch` (desktop tests), but the two were not run together in a real window.

## Notes from implementation

- Spike result recorded in [system/05](../system/05-dashboard.md#content-security-policy).
- `crates/corvus-desktop`: lib + thin bin; `dispatch()` validates method (GET/POST/PUT) and an `/api/…` path,
  caps bodies at 256 KiB, and calls `data_router()` in memory; one `APP_COMMANDS` list feeds the build-time
  allow-list, the capability and the invoke handler. The main window is built in code so `on_navigation` and
  `on_new_window` guards apply.
- The bundle scan strips only the literal comparison `!== 'innerHTML'`/`'outerHTML'` (a deny-list check inside
  Svelte's attribute setter); any use of a sink still fails.
- The "Connect an agent" hook snippet follows the confirmed Claude Code contract (UserPromptSubmit, PostToolUse,
  Stop; agent name set in the command).
- `make test` builds `ui/` and runs its unit tests and bundle scan; `make test-e2e` runs Playwright (WebKit +
  Chromium) and the GUI no-listener test; `make lint` adds the frontend lint/typecheck, the generated-types drift
  check and a check that `corvus-cli` does not depend on the desktop toolkit.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0020 | The dashboard exists only in the self-contained Tauri app: Svelte 5 frontend, one IPC command into the HTTP route table in memory, no browser UI, no `/ui` route, and the app never opens a listener. A spike confirms the bundle is sink-free, with vanilla TypeScript as the fallback. | [0020](../decisions/0020-desktop-only-dashboard.md) |
