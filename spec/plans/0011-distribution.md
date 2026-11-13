# Plan 0011 — Distribution: release binaries and the desktop bundle

- **Status:** Ready
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

- [ ] 1. Release profile (LTO, strip, `panic` strategy reviewed against the hook's catch-all); `corvus --version`
      with version, git commit and schema version.
- [ ] 2. **Dependency audit for self-containment:** the release binary links no system libraries beyond the
      platform's C runtime (SQLite bundled, TLS via rustls); checked with `otool -L` / `ldd` in CI.
- [ ] 3. `cargo install --path crates/corvus-cli` (and from git) works with no Node and no database installed
      (the CLI binary contains no frontend).
- [ ] 4. Build matrix: macOS arm64 and x86_64, Linux x86_64 and arm64 (decide glibc vs musl); archives with
      checksums.
- [ ] 5. Desktop bundle: Tauri build (the one step that needs Node, on the release machine only) embedding the `corvus` binary as a bundled resource; "Install
      command-line tool" links it into `~/.local/bin`; "Connect an agent" shows its real path.
- [ ] 6. macOS signing: sign the `corvus` binary and the app bundle (including the bundled binary) with the
      owner's personal **development** certificate, selected by name from the keychain via an environment
      variable at release time — no certificate, key or password in the repo, logs or build scripts. This is
      enough for testing on the owner's machines. No notarisation in v1; the README documents the first-launch
      Gatekeeper override for any other Mac.
- [ ] 7. Clean-machine smoke test, automated for Linux (a throwaway container with only the binary copied in —
      a development-time check, run with `podman`): `corvus health`, a claim race across processes, an MCP
      handshake. Manual equivalent on macOS.
- [ ] 8. Upgrade path test: install an older build, create data, replace the binary → migrates on open with a
      backup; an old still-running process reports the upgrade clearly instead of misbehaving.
- [ ] 9. Release checklist in `docs/` and (if the owner wants it) a release workflow.

## Verification

- [ ] The release binary runs on a clean Linux image with no packages added: `health`, `task create`, `claim`,
      `mcp` handshake, `hook` — all succeed.
- [ ] Same by hand on a clean macOS user account, for both the binary and the desktop app.
- [ ] `ldd` / `otool -L` output contains no unexpected libraries.
- [ ] `cargo install` from a fresh clone succeeds on a machine without Node.
- [ ] The desktop app, launched on a machine that has never had Corvus, shows a working dashboard and can
      install the CLI; an agent configured with the shown snippet connects.
- [ ] Old binary + newer database → clear refusal. New binary + older database → backup, migrate, run.
- [ ] `codesign --verify --deep --strict` passes on the binary and the app; the app launches on the owner's Mac; the
      documented first-launch override is confirmed to work.
- [ ] No signing material appears in the repository or in build output.
- [ ] Binary size and cold-start time of `corvus hook` are recorded (regression baseline).

## Exit criteria

- [ ] Artifacts for macOS (arm64, x86_64) and Linux (x86_64, arm64) exist with checksums and pass the smoke
      tests; macOS artifacts are signed.
- [ ] The README quick start is "download, run" — no prerequisites section beyond the OS.

## Open questions

- **Linux libc**: static musl (most portable; check rusqlite/reqwest/tauri implications — the desktop app
  cannot be musl-static, so this applies to `corvus` only) vs glibc with a baseline version. Decide at step 4;
  promote to a decision record if contested.
- **Wider distribution**: a development certificate cannot be notarised. Sharing the app beyond the owner's
  machines without a Gatekeeper override needs a Developer ID certificate — a future plan, not v1.
- **Where releases live** (GitHub Releases is the obvious default) and whether to publish to crates.io.

## Unverified / deferred

—

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0013 | SQLite is compiled into the binary, so there is no database to install or start. | [0013](../decisions/0013-storage-sqlite.md) |
| D0015 | Everything is one `corvus` binary — CLI, MCP server, hook, supervisor, HTTP server — so distribution is one file. | [0015](../decisions/0015-process-topology-single-binary.md) |
| D0020 | The desktop app is the only dashboard; it embeds the core and bundles the binary, so it is a complete install on its own and the CLI binary carries no frontend. | [0020](../decisions/0020-desktop-only-dashboard.md) |
