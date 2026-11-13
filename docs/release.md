# Release checklist

Corvus ships as one `corvus` binary (CLI, MCP server, hook, watcher, HTTP API) and, optionally, the desktop app,
which contains that same binary. Nothing else is installed or started on the user's machine.

## 1. Gates

```sh
make lint && make test && make test-e2e        # all must pass
```

## 2. The `corvus` binary

| Platform | Build | Notes |
|---|---|---|
| macOS arm64 | `make release` on an Apple-silicon Mac | links only `libSystem` and `libiconv` (both part of macOS) |
| macOS x86_64 | `rustup target add x86_64-apple-darwin` then `cargo build --release --locked -p corvus-cli --target x86_64-apple-darwin` | |
| Linux arm64 / x86_64 | static musl, built in a throwaway container (development tooling only — `podman`): `rust:1-slim` + `musl-tools`, `rustup target add <arch>-unknown-linux-musl`, `cargo build --release --locked -p corvus-cli --target <arch>-unknown-linux-musl` | fully static (`ldd`: "not a dynamic executable") |

Check every binary:

```sh
corvus --version          # version, git commit, schema version (build where git is available)
otool -L corvus           # macOS: libSystem + libiconv only
ldd corvus                # Linux: "not a dynamic executable"
shasum -a 256 corvus-*    # publish the checksums next to the archives
```

Clean-machine smoke test (Linux): copy only the binary into `debian:stable-slim` and run `health`, a
cross-process claim race, an MCP `initialize` + `tools/list`, and the hook. See plan 0011 for the script used.

## 3. The desktop app (macOS)

```sh
make bundle                                   # needs Node on the release machine only
CORVUS_SIGN_IDENTITY="<keychain identity name>" make bundle   # signed
```

`make bundle` builds the release `corvus`, embeds it as the app's sidecar (`Corvus.app/Contents/MacOS/corvus`,
which "Install command-line tool" links into `~/.local/bin`), and builds `target/release/bundle/macos/Corvus.app`.
With `CORVUS_SIGN_IDENTITY` set it signs the bundled `corvus` and the app with that identity from the keychain
and runs `codesign --verify --deep --strict`. The identity is chosen at release time; no certificate, key or
password is ever stored in the repository, scripts or logs.

Development certificates cannot be notarised. On a Mac other than the signer's, the first launch needs the
Gatekeeper override: right-click `Corvus.app` → Open → Open (or System Settings → Privacy & Security → Open
Anyway).

## 4. Upgrades

A newer `corvus` migrates an older database on first open, writing `corvus.db.pre-v<N>.bak` first. An older
`corvus` refuses a newer database with "upgrade Corvus". A process that was already running when the schema
moved reports "Corvus was upgraded — restart this process" on its next write.

## 5. Publish

Archives + `SHA256SUMS` for macOS arm64/x86_64 and Linux arm64/x86_64, plus `Corvus.app` zipped. Where releases
live (GitHub Releases is the default) is the owner's call.
