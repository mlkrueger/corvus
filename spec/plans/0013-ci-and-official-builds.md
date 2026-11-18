# Plan 0013 — CI and official builds in GitHub Actions

- **Status:** In progress
- **Depends on:** [0011](0011-distribution.md)
- **Spec:** [system/11](../system/11-defaults.md) rows 34, 56–58 · [docs/release.md](../../docs/release.md)

## Goal

Every push is linted and tested by GitHub Actions, and pushing a version tag produces the signed release
artifacts and the GitHub Release with no step on a developer machine.

## Scope

**In:** the public repository's licence; a CI workflow (lint, tests); a release workflow (macOS binaries and
app, Linux binaries, smoke tests, checksums, GitHub Release); signing with the owner's self-signed certificate
from GitHub secrets; the `release` environment; `docs/release.md`.

**Out:** auto-update in the app and how the `corvus` command is kept in step with it (a later plan);
notarisation / Developer ID ([D0026](../decisions/0026-official-builds-in-github-actions.md)); an x86_64 build
of the desktop app; browser-engine and GUI tests in CI (`make test-e2e` stays a local gate).

## Work breakdown

- [x] 1. Licence: Apache-2.0 (`LICENSE`, `NOTICE`, workspace manifest, README).
- [x] 2. `ci.yml`: `make lint` and `make test` on macOS; the workspace minus the desktop app on Linux.
- [x] 3. `release.yml`: tag-triggered; macOS job (`make dist-macos dist-app`, certificate imported into a
        throwaway keychain), Linux matrix (`make native-linux-<arch> smoke-linux-<arch>`), publish job.
        New Makefile target `native-linux-%`.
- [x] 4. Decision record, defaults rows 57 and 58, `docs/release.md`.
- [x] 5. Push; get `ci.yml` green on both jobs.
- [ ] 6. Create the `release` environment (owner approval required); owner exports the certificate and sets
        `MACOS_CERT_P12`, `MACOS_CERT_PASSWORD` and the variable `CORVUS_SIGN_IDENTITY`.
- [ ] 7. Manual run of `release.yml` (no publish): all jobs green, artifacts inspected.
- [ ] 8. First tagged release from the workflow.

## Verification

- [x] `actionlint` passes on both workflows.
- [x] CI green on a push: macOS (`make lint`, `make test`) and Linux (run 44267004775, 2026-11-18).
- [ ] Manual release run: five archives; macOS artifacts show the expected `Authority`; both Linux smoke tests
      pass on native runners.
- [ ] Tagged run with the certificate missing fails before building.
- [ ] Tagged run: the GitHub Release holds the five archives and `SHA256SUMS`; a downloaded app opens after the
      Gatekeeper override; a downloaded Linux binary passes `corvus health`.

## Exit criteria

- [ ] `make lint` and `make test` pass locally and in CI.
- [ ] One release published entirely by the workflow.

## Open questions

- Does `codesign` accept the self-signed identity from a throwaway keychain on a runner without a trust
  setting? Answered by step 7.
- **Resolved in step 5:** no test depended on timing or tools the runners lack; the suite passed on both.

## Unverified / deferred

- `ci.yml` has run green. `release.yml` has not run yet; the `release` environment exists (owner approval
  required) but holds no certificate.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0026 | Official builds are made by GitHub Actions on a version tag and signed with the owner's self-signed certificate from GitHub secrets; a tagged build without the certificate fails. Local builds are for testing only. | [link](../decisions/0026-official-builds-in-github-actions.md) |
