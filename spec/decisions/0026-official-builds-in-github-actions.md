# D0026 — Official builds are made by GitHub Actions, not on a developer machine

- **Status:** Accepted
- **Date:** 2026-11-18
- **Plans:** [0013](../plans/0013-ci-and-official-builds.md), [0014](../plans/0014-app-updates.md)
- **Spec:** [system/11](../system/11-defaults.md) rows 34, 56–58 · [docs/release.md](../../docs/release.md)

## Problem

Until now a release was whatever `make dist` produced on the owner's Mac ([plan 0011](../plans/0011-distribution.md)).
The repository is now public and releases will be downloaded by other people and, later, fetched by an
updater. Where the published artifacts are built, and with which signature, has to be settled before the
first public release.

## Options

1. **Keep building on the owner's Mac** and upload by hand.
2. **Build in GitHub Actions on a version tag**, signing the macOS artifacts with the owner's self-signed
   certificate held in GitHub secrets.
3. **Build in GitHub Actions and sign with an Apple Developer ID**, notarised.
4. **Build in GitHub Actions, unsigned.**

## Trade-offs

### 1. Owner's Mac
- **For:** nothing to set up; the signing key never leaves the keychain.
- **Against:** a build can include uncommitted or machine-specific state; nobody can see how an artifact was
  made; every release depends on one laptop, podman and a manual checklist; an updater needs releases that are
  produced identically each time.

### 2. Actions, self-signed
- **For:** every artifact comes from a tagged commit on a clean runner with a public log; Linux binaries are
  built natively instead of cross-compiled; free for a public repository; the same signing identity as the
  builds already installed, so later versions keep it.
- **Against:** the certificate's private key must be exported into GitHub secrets; macOS still shows the
  first-launch Gatekeeper warning because nobody else's Mac trusts a self-signed certificate.

### 3. Actions, Developer ID
- **For:** as 2, and the app opens with no warning.
- **Against:** USD 99 a year to distribute free software.

### 4. Actions, unsigned
- **For:** no secrets at all.
- **Against:** no stable identity across versions; the same Gatekeeper warning as 2 with nothing gained.

## Decision

**We chose option 2.** Owner direction (2026-11-18): official builds must not be made locally, and paying for a
Developer ID to give software away is not justified for now. Building in Actions gives provenance and
repeatability; the self-signed certificate keeps one identity across versions at no cost.

- A tag `v<version>` runs [`release.yml`](../../.github/workflows/release.yml): macOS binaries and the app,
  Linux binaries for both architectures (each smoke-tested), `SHA256SUMS`, then a GitHub Release.
- A tagged build with no signing certificate configured **fails**; it never publishes unsigned artifacts. A
  manual run builds without publishing and may be unsigned.
- Secrets live in a GitHub environment named `release`, which only the macOS build job and the publish job
  use. Pull requests and ordinary pushes run [`ci.yml`](../../.github/workflows/ci.yml), which has no secrets.
- Actions are pinned to commit hashes.
- `make dist` stays, for building the same set locally when testing; its output is not a release.

## Consequences

- The self-signed certificate's private key exists in GitHub's secret store as well as the owner's keychain.
  If it leaks, anyone can sign as that identity; since no Mac trusts the identity, the damage is limited to
  users who already chose to trust a Corvus build.
- First launch on any Mac still needs the Gatekeeper override.
- Revisit when Corvus has users for whom the warning is a real barrier: moving to a Developer ID changes the
  signing identity once (option 3) and needs no other change to the workflow's shape.

## Discussion

<!-- Append dated entries; never rewrite earlier ones. -->
