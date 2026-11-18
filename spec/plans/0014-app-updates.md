# Plan 0014 — App updates from GitHub Releases

- **Status:** In progress
- **Depends on:** [0013](0013-ci-and-official-builds.md)
- **Spec:** [system/05](../system/05-dashboard.md#desktop-app-cratescorvus-desktop-tauri-2) · [system/08](../system/08-security.md#surfaces) · [system/10](../system/10-config-ops.md)

## Goal

A running desktop app notices a newer release, offers "Install and restart", and after one click is the new
version, with the `corvus` command on `PATH` moved along with it.

## Scope

**In:** the update check and install in the app (Rust, behind two commands); the banner in the dashboard; the
`CORVUS_UPDATE_CHECK` switch; the signed update archive and `latest.json` in the release workflow; the updater
key; documentation of the app-owned `corvus` link.

**Out:** updating stand-alone `corvus` archives; Linux and Intel builds of the app; silent installs; release
notes beyond the version; detecting a second, older `corvus` earlier on the user's `PATH` (a GUI app does not
see the shell's `PATH`; version skew is already reported by the database check).

## Work breakdown

- [x] 1. Spec: decision D0027; system 05, 08, 10; defaults.
- [x] 2. `CORVUS_UPDATE_CHECK` in `Config`.
- [x] 3. App: updater plugin registered with no page permission; `update` module (status, six-hour cache,
        install → stop background work, checkpoint, restart); commands `check_update`, `install_update`.
- [x] 4. Dashboard: banner with the version and "Install and restart"; "Check for updates" in the Connect view.
- [x] 5. Updater keypair; public key in `tauri.conf.json`.
- [x] 6. Release: `make dist-app` writes the signed update archive and `latest.json` when the key is present;
        the workflow requires the key on a tag and publishes both.
- [x] 7. Docs: `docs/release.md`, `docs/usage.md`, README.
- [ ] 8. By hand: publish two versions; an installed first version offers and installs the second.

## Verification

- [x] Config: default on; `off`/`on` parsed; anything else rejected.
- [x] Cache: a second check inside six hours does not call out; `force` does; "disabled" never does.
- [x] Capability test: the page is granted exactly the six app commands and no plugin permission.
- [x] `tauri.conf.json` version equals the workspace version.
- [x] Manifest script: version, platform key, URL and signature land where the updater expects them.
- [x] UI unit tests: banner hidden with no update, shown with one; install error shown; disabled state.
- [ ] By hand: step 8; `CORVUS_UPDATE_CHECK=off` makes no connection; the linked `corvus --version` reports the
      new version after the update.

## Exit criteria

- [x] `make lint` and `make test` pass (581 Rust tests, 129 UI tests; see the one flake below).
- [ ] One real update installed from a published release.

## Open questions

- Does `AppHandle::restart` run the app's exit handling? The install path stops background work and checkpoints
  itself first, so the answer does not matter for correctness.

## Unverified / deferred

- **No update has been installed yet.** Built and unit-tested: the check memory, the commands, the banner, the
  manifest script. Checked by hand: `make dist-app` with the key writes the archive, its `.sig` and
  `latest.json`; the archive unpacks to a working app with no AppleDouble files. Not exercised: the request to
  GitHub from a running app, download, signature verification, replacing the app and the restart. That needs
  two published releases (step 8).
- The install is driven from the dashboard only; there is no tray menu entry for it.
- `admin_backup_is_consistent_while_another_process_writes` (corvus-cli `foundation` tests) failed once during
  `make test` while the UI build was running alongside, then passed in the full suite and six times alone. Not
  caused by this plan as far as seen; the failing command's error was not captured.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0027 | The app checks GitHub Releases, offers the update and installs it only on a click, verifying it against an updater key; `CORVUS_UPDATE_CHECK=off` disables it. The `corvus` command on `PATH` is the app's link into its bundle, so one update moves both. | [link](../decisions/0027-app-updates-from-github-releases.md) |
| D0026 | Official builds, including the update archive and manifest, are made by GitHub Actions on a version tag. | [link](../decisions/0026-official-builds-in-github-actions.md) |
