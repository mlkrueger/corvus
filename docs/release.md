# Release checklist

Corvus ships as one `corvus` binary (CLI, MCP server, hook, watcher, HTTP API) and, optionally, the desktop app,
which contains that same binary. Nothing else is installed or started on the user's machine.

## 1. Gates

```sh
make lint && make test && make test-e2e        # all must pass
```

## 2. Build everything

```sh
CORVUS_SIGN_IDENTITY="<keychain identity name>" make dist
```

`make dist` writes into `target/dist/`:

| Artifact | How |
|---|---|
| `corvus-<v>-aarch64-apple-darwin.tar.gz`, `corvus-<v>-x86_64-apple-darwin.tar.gz` | native builds; both signed when `CORVUS_SIGN_IDENTITY` is set; the x86_64 one is run under Rosetta |
| `corvus-<v>-aarch64-unknown-linux-musl.tar.gz`, `corvus-<v>-x86_64-unknown-linux-musl.tar.gz` | static musl, cross-compiled in a throwaway `podman` container of this machine's architecture |
| `Corvus-<v>-macos-<arch>.app.zip` | `make bundle` (needs Node on the release machine only), zipped with `ditto` so the signature survives |
| `SHA256SUMS` | over every archive |

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
publish. Tag the commit the artifacts were built from, then upload everything in `target/dist/`:

```sh
git tag v<v> && git push origin v<v>
gh release create v<v> target/dist/*.tar.gz target/dist/*.zip target/dist/SHA256SUMS --title "Corvus <v>" --notes-file <notes>
```
