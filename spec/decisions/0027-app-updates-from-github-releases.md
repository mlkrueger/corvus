# D0027 — The desktop app updates itself from GitHub Releases, when the owner says so; the app owns the `corvus` command

- **Status:** Accepted
- **Date:** 2026-11-18
- **Plans:** [0014](../plans/0014-app-updates.md)
- **Spec:** [system/05](../system/05-dashboard.md#desktop-app-cratescorvus-desktop-tauri-2) · [system/08](../system/08-security.md#surfaces) · [system/10](../system/10-config-ops.md)

## Problem

A new version reaches a machine only if someone downloads it and replaces the app by hand
([plan 0011](../plans/0011-distribution.md) left updating out). Releases are now built and published by a
workflow ([D0026](0026-official-builds-in-github-actions.md)), so the app can find them. Three things must be
settled: where the app looks, how much it does on its own, and how the `corvus` command that hooks and tool
configurations call stays the same version as the app.

Constraints: Corvus is local-first and self-contained (no service of its own to run, nothing required at run
time); the page has no network access and no plugin permissions; an update restarts the background leader and
may migrate the database.

## Options

**How the app is updated**

1. **Manual only** — as today.
2. **Notify only** — the app says a version exists and links to the release page.
3. **Check, offer, install on a click** — the app checks GitHub Releases, shows "Install and restart", verifies
   the download against a key built into the app, replaces itself and restarts.
4. **Silent automatic install.**

**How the `corvus` command is kept in step**

- a. **The app owns it** — the command on `PATH` is a link into the app bundle; one update moves both.
- b. **`corvus update`** — the command updates itself from the same release.
- c. **Notice only** — the command reports that a newer version exists.

## Trade-offs

### 1. Manual only
- **For:** no network call, no key to manage.
- **Against:** versions drift; every fix needs every user to act.

### 2. Notify only
- **For:** no install code path; no updater key.
- **Against:** the user still downloads, replaces and overrides Gatekeeper again each time (a browser download
  is quarantined; an in-app install is not).

### 3. Check, offer, install on a click
- **For:** one click; the download is verified against a key that is independent of the Apple signing identity,
  so it works with the self-signed certificate; the owner chooses the moment of the restart; nothing to host —
  the manifest is a file on the release.
- **Against:** the first outbound connection Corvus makes by default (to `github.com`); a second key to keep;
  one plugin added to the app.

### 4. Silent automatic install
- **For:** nobody stays behind.
- **Against:** restarts the background leader mid-work and may migrate the database with no one watching;
  surprising for a local tool.

### a. The app owns the command
- **For:** one update path; uses the existing "Install command-line tool" link, which points into the bundle
  and so follows every update with no further step.
- **Against:** the command is only updated on machines that run the app (macOS).

### b. `corvus update`
- **For:** works without the app, including Linux.
- **Against:** a second update path with its own download, verification and replace-in-place logic.

### c. Notice only
- **For:** trivial.
- **Against:** leaves the two versions apart, and a command older than the app's database refuses to run.

## Decision

**We chose option 3 with option a.** Owner direction (2026-11-18).

- **Where:** `https://github.com/mlkrueger/corvus/releases/latest/download/latest.json`, written by the release
  workflow, naming the version and the signed update archive for Apple silicon macOS.
- **When:** once after launch and at most every six hours while the app runs; also on request. A failed check
  is logged and shown only when the owner asked for it.
- **What the app does alone:** checks. It never downloads or installs without a click on "Install and restart".
- **Verification:** the archive must carry a signature made with the updater key; the public key is in the
  app's configuration. The private key exists only as a GitHub secret of the `release` environment and in the
  owner's own backup.
- **Off switch:** `CORVUS_UPDATE_CHECK=off` — no check, no outbound connection, the commands answer "disabled".
- **Isolation is unchanged:** the updater runs in Rust behind two app commands (`check_update`,
  `install_update`); the page gets no plugin permission and no network access.
- **The command:** the supported way to have `corvus` on `PATH` on a Mac with the app is the app's link
  (`~/.local/bin/corvus` → the bundle). `corvus setup claude` writes the path it was started from, so hooks set
  up through the link follow app updates. Stand-alone archives (Linux, or macOS without the app) are replaced by
  hand; nothing checks for them.

## Consequences

- A default install of the app now makes one outbound HTTPS request to GitHub at launch and periodically
  ([system/08](../system/08-security.md#surfaces)). The `corvus` binary alone still makes none.
- Losing the updater private key means installed apps can never be updated in place again; a leak lets someone
  who can also alter a release ship a malicious update. Both are owner-held risks.
- Every release must ship `latest.json` and the signed archive, and `tauri.conf.json`'s version must equal the
  workspace version; the workflow and a test enforce both.
- An app run from a quarantined, translocated location cannot replace itself; it must be in `Applications`.
- Revisit option b if Linux users of the stand-alone binary appear in numbers.

## Discussion

<!-- Append dated entries; never rewrite earlier ones. -->
