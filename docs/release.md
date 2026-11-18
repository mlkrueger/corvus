# Release checklist

Corvus ships as one `corvus` binary (CLI, MCP server, hook, watcher, HTTP API) and, optionally, the desktop app,
which contains that same binary. Nothing else is installed or started on the user's machine.

## 1. Gates

```sh
make lint && make test && make test-e2e        # all must pass
```

## 2. Build everything

Official builds are made by GitHub Actions, never on a developer machine
([D0026](../spec/decisions/0026-official-builds-in-github-actions.md)). Pushing a tag `v<version>` that matches
the version in `Cargo.toml` runs [`release.yml`](../.github/workflows/release.yml), which builds every artifact
below, signs the macOS ones and publishes the GitHub Release (section 5). Running the workflow by hand
(Actions → Release → Run workflow) builds the same artifacts without publishing.

To build the same set locally for testing (not a release):

```sh
CORVUS_SIGN_IDENTITY="<keychain identity name>" make dist
```

`make dist` writes into `target/dist/`:

| Artifact | How |
|---|---|
| `corvus-<v>-aarch64-apple-darwin.tar.gz`, `corvus-<v>-x86_64-apple-darwin.tar.gz` | native builds; both signed when `CORVUS_SIGN_IDENTITY` is set; the x86_64 one is run under Rosetta |
| `corvus-<v>-aarch64-unknown-linux-musl.tar.gz`, `corvus-<v>-x86_64-unknown-linux-musl.tar.gz` | static musl; in the workflow built natively on a Linux runner of each architecture (`make native-linux-<arch>`), locally cross-compiled in a throwaway `podman` container |
| `Corvus-<v>-macos-<arch>.app.zip` | `make bundle` (needs Node on the release machine only), zipped with `ditto` so the signature survives |
| `SHA256SUMS` | over every archive |

The workflow builds the app for Apple silicon only.

Each Linux archive is smoke-tested in a stock `debian:stable-slim` container holding only the binary
(`make smoke-linux-arm64` / `smoke-linux-amd64`): static linkage, `health`, an 8-process claim race with exactly
one winner, an MCP `initialize` + `tools/list`, and the hook. Any failure stops `make dist`.

Each archive holds `corvus` and `README.md`. Check by hand if in doubt:

```sh
corvus --version          # version, git commit, schema version
otool -L corvus           # macOS: libSystem + libiconv only
codesign -dv corvus       # macOS: Authority = your identity
```

## 3. The desktop app (macOS)

`make bundle` builds the release `corvus`, embeds it as the app's sidecar (`Corvus.app/Contents/MacOS/corvus`,
which "Install command-line tool" links into `~/.local/bin`), and builds `target/release/bundle/macos/Corvus.app`.
With `CORVUS_SIGN_IDENTITY` set it signs the bundled `corvus` and the app with that identity from the keychain
and runs `codesign --verify --deep --strict`. The identity is chosen at release time; no certificate, key or
password is ever stored in the repository, scripts or logs.

The identity in use is the owner's self-signed personal certificate. It cannot be notarised, and macOS does not
trust it, so on any Mac the first launch needs the Gatekeeper override: right-click `Corvus.app` → Open → Open (or
System Settings → Privacy & Security → Open Anyway). Later versions signed with the same certificate keep the
same identity.

## 4. Upgrades

A newer `corvus` migrates an older database on first open, writing `corvus.db.pre-v<N>.bak` first. An older
`corvus` refuses a newer database with "upgrade Corvus". A process that was already running when the schema
moved fails its next write with code `upgraded`: "Corvus was upgraded — restart this process".

## 5. Publish

Releases live on GitHub Releases ([defaults](../spec/system/11-defaults.md) row 56); there is no crates.io
publish. With the gates green on the commit to release:

```sh
git tag v<v> && git push origin v<v>
```

The workflow waits for the owner's approval of the `release` environment, builds, and creates the release
"Corvus <v>" with every archive and `SHA256SUMS`. A tag that does not match `Cargo.toml`, or a tagged build with
no signing certificate configured, fails before anything is published.

## 6. One-time setup of signing in GitHub

The signing certificate lives in the `release` environment (Settings → Environments), which only the release
workflow's macOS and publish jobs can read; pull requests cannot.

1. In Keychain Access, export the signing certificate **with its private key** as a `.p12` and choose an export
   password.
2. Store it, from the folder holding the file:

   ```sh
   base64 -i <file>.p12 | gh secret set MACOS_CERT_P12 --env release
   gh secret set MACOS_CERT_PASSWORD --env release          # prompts for the export password
   gh variable set CORVUS_SIGN_IDENTITY --env release --body "<certificate name>"
   ```

3. Delete the `.p12` file.
