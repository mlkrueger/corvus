#!/bin/sh
# Corvus release check for a clean Mac (plans 0011, 0013, 0014).
#
#   curl -fsSL https://raw.githubusercontent.com/mlkrueger/corvus/build/v1/tools/release-check.sh | sh
#   sh tools/release-check.sh            # the latest release
#   sh tools/release-check.sh v0.1.0     # a given tag
#
# Downloads the release from GitHub, checks SHA256SUMS and the signatures, marks the downloads as quarantined
# (the way a browser download is), installs Corvus.app into /Applications, and unpacks the standalone `corvus`
# into ~/corvus-release-check/bin (kept off PATH, so the app's "Install command-line tool" link is what gets
# tested). Needs no sudo, Homebrew, Rust or Node. Safe to run again for a newer tag, except that it will not
# replace an app that is already installed: that is the in-app update's job (plan 0014).
set -eu

REPO="mlkrueger/corvus"
WORK="$HOME/corvus-release-check"
APPS="${CORVUS_APPS_DIR:-/Applications}"   # for testing this script only

say() { printf '\n==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || die "this script is for macOS"
case "$(uname -m)" in
  arm64) TARGET=aarch64-apple-darwin; APP_ARCH=arm64 ;;
  x86_64) TARGET=x86_64-apple-darwin; APP_ARCH="" ;;
  *) die "unknown architecture $(uname -m)" ;;
esac

TAG="${1:-}"
if [ -z "$TAG" ]; then
  # The redirect of /releases/latest names the newest published tag.
  TAG=$(curl -fsSI "https://github.com/$REPO/releases/latest" | tr -d '\r' |
    sed -n 's|^[Ll]ocation: .*/releases/tag/||p')
  [ -n "$TAG" ] || die "no published release found for $REPO"
fi
VERSION="${TAG#v}"
BASE="${CORVUS_RELEASE_BASE:-https://github.com/$REPO/releases/download/$TAG}"
say "Corvus $VERSION ($TAG) for $(uname -m)"

DL="$WORK/download-$VERSION"
rm -rf "$DL"
mkdir -p "$DL" "$WORK/bin"
cd "$DL"

BIN_TGZ="corvus-$VERSION-$TARGET.tar.gz"
APP_ZIP="Corvus-$VERSION-macos-$APP_ARCH.app.zip"

say "Downloading"
curl -fL --progress-bar -o SHA256SUMS "$BASE/SHA256SUMS"
curl -fL --progress-bar -o "$BIN_TGZ" "$BASE/$BIN_TGZ"
if [ -n "$APP_ARCH" ]; then
  curl -fL --progress-bar -o "$APP_ZIP" "$BASE/$APP_ZIP"
else
  echo "(the app is built for Apple silicon only; this Intel Mac gets the binary only)"
fi

say "Checking SHA256SUMS"
for f in "$BIN_TGZ" ${APP_ARCH:+"$APP_ZIP"}; do
  want=$(awk -v f="$f" '$2 == f || $2 == "*"f {print $1}' SHA256SUMS)
  [ -n "$want" ] || die "$f is not listed in SHA256SUMS"
  have=$(shasum -a 256 "$f" | awk '{print $1}')
  [ "$want" = "$have" ] || die "checksum mismatch for $f"
  echo "ok  $f"
done

# curl does not set the quarantine flag that Safari/Chrome set; add it so Gatekeeper behaves as for a
# browser download. CORVUS_NO_QUARANTINE=1 skips this.
quarantine() {
  [ "${CORVUS_NO_QUARANTINE:-0}" = 1 ] && return 0
  xattr -w com.apple.quarantine "0083;$(printf '%x' "$(date +%s)");Safari;" "$1"
}

say "Standalone binary -> $WORK/bin/corvus"
tar -xzf "$BIN_TGZ" -C "$WORK/bin" corvus
quarantine "$WORK/bin/corvus"
codesign --verify --strict "$WORK/bin/corvus" && echo "codesign --verify --strict: ok"
codesign -dvv "$WORK/bin/corvus" 2>&1 | grep -E '^(Authority|Identifier|TeamIdentifier)=' || true
otool -L "$WORK/bin/corvus" | sed 1d

if [ -n "$APP_ARCH" ]; then
  say "Desktop app -> "$APPS/Corvus.app""
  if [ -e "$APPS/Corvus.app" ]; then
    echo "$APPS/Corvus.app already exists; leaving it alone (use the in-app update, or delete it first)."
  else
    # ditto keeps the signature and extended attributes intact (unzip can break the bundle's signature).
    ditto -x -k "$APP_ZIP" "$DL/app"
    ditto "$DL/app/Corvus.app" "$APPS/Corvus.app"
    quarantine "$APPS/Corvus.app"
  fi
  codesign --verify --deep --strict "$APPS/Corvus.app" && echo "codesign --verify --deep --strict: ok"
  codesign -dvv "$APPS/Corvus.app" 2>&1 | grep -E '^(Authority|Identifier)=' || true
  echo "Gatekeeper's verdict before the override (a rejection is expected for a personal certificate):"
  spctl --assess --type execute -vv "$APPS/Corvus.app" 2>&1 || true
  echo "quarantine flag: $(xattr -p com.apple.quarantine "$APPS/Corvus.app" 2>/dev/null || echo none)"
fi

say "Existing Corvus state on this Mac (should be none for the clean-machine check)"
for p in "$HOME/.corvus" "$HOME/.local/bin/corvus" "$HOME/Library/LaunchAgents/dev.corvus.desktop.plist"; do
  if [ -e "$p" ]; then echo "present: $p"; else echo "absent:  $p"; fi
done

cat <<EOF

==> Downloaded and verified. Now, by hand:

Binary (plan 0011):
  1. $WORK/bin/corvus --version
     Expect macOS to block it (quarantine). Allow it under System Settings > Privacy & Security, run it
     again, and note what the dialog said.
  2. $WORK/bin/corvus health          # {"ok":true,...}

App, Gatekeeper and first launch (plans 0013, 0011):
  3. Double-click "$APPS/Corvus.app": expect a block. Then right-click > Open > Open (or Privacy &
     Security > Open Anyway). Note which override worked.
  4. The dashboard opens and works. Connect an agent > Install command-line tool, then:
       ~/.local/bin/corvus --version    (should report $VERSION)
  5. Point an agent at the MCP snippet shown in the app, for example:
       claude mcp add corvus -- ~/.local/bin/corvus mcp
     then check that it connects: ~/.local/bin/corvus session list
  6. Login item: ls ~/Library/LaunchAgents/dev.corvus.desktop.plist

Update (plan 0014), once a newer release is published:
  7. Keep the app running (or relaunch it): "Install and restart" should appear, or use Check for updates.
     Install it, then: ~/.local/bin/corvus --version   (the new version, through the same link)
  8. Quit the app, start it with checks off, and confirm it makes no connection to GitHub:
       CORVUS_UPDATE_CHECK=off "$APPS/Corvus.app"/Contents/MacOS/corvus-desktop
     (in another terminal: lsof -i -nP | grep -i corvus  # expect nothing)

Clean up afterwards with: rm -rf "$WORK"
EOF
