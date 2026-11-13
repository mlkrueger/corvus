# Plan 0011 — Distribution: release binaries and the desktop bundle

- **Status:** In progress
- **Depends on:** [0006](0006-dashboard-and-desktop.md), [0007](0007-delivery-helpers.md)
- **Spec:** [system/01](../system/01-architecture.md#artifacts) · owner direction (self-contained binary, no containers)

## Goal

Someone with a clean machine can obtain Corvus as a single binary — or as the desktop app — and it works, with
nothing else to install, start or configure.

## Scope

**In:** release build profile; `cargo install` path; prebuilt `corvus` binaries for macOS and Linux; the Tauri
bundle with the `corvus` binary inside it and the "Install command-line tool" action; macOS code signing with
the owner's personal identity; version reporting; clean-machine smoke tests; release checklist.

**Out:** anything Windows-related (owner direction — not deferred, excluded); auto-update; package-manager
formulas (Homebrew, apt); signing under an organisation identity.

## Work breakdown

- [x] 1. Release profile (LTO, strip, `panic` strategy reviewed against the hook's catch-all); `corvus --version`
      with version, git commit and schema version.
- [x] 2. **Dependency audit for self-containment:** the release binary links no system libraries beyond the
      platform's C runtime (SQLite bundled, TLS via rustls); checked with `otool -L` / `ldd` in CI.
- [x] 3. `cargo install --path crates/corvus-cli` (and from git) works with no Node and no database installed
      (the CLI binary contains no frontend).
- [ ] 4. Build matrix: macOS arm64 and x86_64, Linux x86_64 and arm64 (decide glibc vs musl); archives with
      checksums.
- [x] 5. Desktop bundle: Tauri build (the one step that needs Node, on the release machine only) embedding the `corvus` binary as a bundled resource; "Install
      command-line tool" links it into `~/.local/bin`; "Connect an agent" shows its real path.
- [ ] 6. macOS signing: sign the `corvus` binary and the app bundle (including the bundled binary) with the
      owner's personal **development** certificate, selected by name from the keychain via an environment
      variable at release time — no certificate, key or password in the repo, logs or build scripts. This is
      enough for testing on the owner's machines. No notarisation in v1; the README documents the first-launch
      Gatekeeper override for any other Mac.
- [x] 7. Clean-machine smoke test, automated for Linux (a throwaway container with only the binary copied in —
      a development-time check, run with `podman`): `corvus health`, a claim race across processes, an MCP
      handshake. Manual equivalent on macOS.
- [ ] 8. Upgrade path test: install an older build, create data, replace the binary → migrates on open with a
      backup; an old still-running process reports the upgrade clearly instead of misbehaving.
- [x] 9. Release checklist in `docs/` and (if the owner wants it) a release workflow.

## Verification

- [x] The release binary runs on a clean Linux image with no packages added: `health`, `task create`, `claim`,
      `mcp` handshake, `hook` — all succeed.
- [ ] Same by hand on a clean macOS user account, for both the binary and the desktop app.
- [x] `ldd` / `otool -L` output contains no unexpected libraries.
- [x] `cargo install` from a fresh clone succeeds on a machine without Node.
- [ ] The desktop app, launched on a machine that has never had Corvus, shows a working dashboard and can
      install the CLI; an agent configured with the shown snippet connects.
- [ ] Old binary + newer database → clear refusal. New binary + older database → backup, migrate, run.
- [ ] `codesign --verify --deep --strict` passes on the binary and the app; the app launches on the owner's Mac; the
      documented first-launch override is confirmed to work.
- [x] No signing material appears in the repository or in build output.
- [x] Binary size and cold-start time of `corvus hook` are recorded (regression baseline).

## Exit criteria

- [ ] Artifacts for macOS (arm64, x86_64) and Linux (x86_64, arm64) exist with checksums and pass the smoke
      tests; macOS artifacts are signed.
- [ ] The README quick start is "download, run" — no prerequisites section beyond the OS.

## Open questions

- **Linux libc** — decided at step 4: static musl for `corvus` (built and smoke-tested for arm64; bundled SQLite
  and rustls compile cleanly). Recorded in [defaults](../system/11-defaults.md) row 55.
- **Wider distribution**: a development certificate cannot be notarised. Sharing the app beyond the owner's
  machines without a Gatekeeper override needs a Developer ID certificate — a future plan, not v1.
- **Where releases live** (GitHub Releases is the obvious default) and whether to publish to crates.io.

## Unverified / deferred

Done on the build machine (macOS 26 arm64, 2026-11-13):
- Release profile (`lto = "thin"`, `codegen-units = 1`, `strip`; `panic` stays `unwind` for the hook's and
  housekeeping's catch-alls). `corvus --version` → `0.1.0 (commit <sha>, schema 6)`, with a test tying the schema
  number to the migrator.
- macOS arm64 release binary: 10.4 MiB; `otool -L` → `libSystem.B.dylib` and `libiconv.2.dylib` only.
- `cargo install --locked --path crates/corvus-cli` from a fresh clone with Node absent from `PATH`: built;
  `corvus health` → `{"ok":true,"schema_version":6}`.
- Linux arm64 static musl binary built in a `rust:1-slim` container (10.6 MiB); in a stock
  `debian:stable-slim` container with only the binary: `ldd` → not dynamic; `health` ok; 8 concurrent claim
  processes → exactly 1 winner; MCP `initialize` + `tools/list` (20 tools); hook injection ok.
- `make bundle` → `Corvus.app` (22.7 MiB) with `corvus` as its sidecar in `Contents/MacOS`, unsigned.
- Baselines (release, macOS arm64): `corvus hook` read path median 3.8 ms / p95 4.8 ms; throttle path median
  2.7 ms / p95 2.9 ms.

**Not done — needs the owner or other machines:**
- **Signing.** No code-signing identity exists in the build machine's keychain. `make bundle` signs when
  `CORVUS_SIGN_IDENTITY` names one; `codesign --verify --deep --strict` and the first-launch override are
  unverified.
- **macOS x86_64 and Linux x86_64 builds** (only the arm64 targets were built; the steps are in
  [docs/release.md](../../docs/release.md)). Archives with checksums are therefore not produced.
- **Clean macOS user account** check for the binary and the app; the app on a machine that never had Corvus.
- **Upgrade path with an older build**: there is only one schema version so far, so "older build" cannot be
  produced; the mechanics (newer DB refused; older DB backed up and migrated; running process told to restart)
  are covered by the plan 0001 tests.
- Where releases live and crates.io publishing — owner's call.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0013 | SQLite is compiled into the binary, so there is no database to install or start. | [0013](../decisions/0013-storage-sqlite.md) |
| D0015 | Everything is one `corvus` binary — CLI, MCP server, hook, supervisor, HTTP server — so distribution is one file. | [0015](../decisions/0015-process-topology-single-binary.md) |
| D0020 | The desktop app is the only dashboard; it embeds the core and bundles the binary, so it is a complete install on its own and the CLI binary carries no frontend. | [0020](../decisions/0020-desktop-only-dashboard.md) |
