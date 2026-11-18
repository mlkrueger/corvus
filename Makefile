# Development conveniences only; nothing here is needed to build or run Corvus.
.PHONY: release bundle ui ui-deps types check-types test test-rust test-ui test-e2e lint lint-ui check-cli-node-free fmt check-purity check-rusqlite check-sql-clock check-seam run

CARGO ?= cargo
NON_CORE_CRATES := corvus-domain corvus-jira corvus-linear corvus-http corvus-mcp corvus-cli corvus-desktop

HOST_TRIPLE := $(shell rustc -vV | sed -n 's/^host: //p')

# The release corvus binary (no Node needed).
release:
	$(CARGO) build --release --locked -p corvus-cli

# The desktop app bundle with the corvus binary inside it (needs Node, on the release machine only).
# Signing: set CORVUS_SIGN_IDENTITY to a keychain identity name; nothing secret lives in the repo.
bundle: release ui
	mkdir -p crates/corvus-desktop/binaries
	cp target/release/corvus crates/corvus-desktop/binaries/corvus-$(HOST_TRIPLE)
	cd crates/corvus-desktop && ../../ui/node_modules/.bin/tauri build --no-sign \
	  --config '{"bundle":{"active":true,"targets":["app"],"externalBin":["binaries/corvus"]},"build":{"beforeBuildCommand":""}}'
	@if [ -n "$$CORVUS_SIGN_IDENTITY" ]; then \
	  app=$$(ls -d target/release/bundle/macos/*.app); \
	  codesign --force --options runtime --sign "$$CORVUS_SIGN_IDENTITY" "$$app/Contents/MacOS/corvus" && \
	  codesign --force --options runtime --sign "$$CORVUS_SIGN_IDENTITY" "$$app" && \
	  codesign --verify --deep --strict "$$app" && echo "signed: $$app"; \
	else echo "unsigned bundle (set CORVUS_SIGN_IDENTITY to sign)"; fi

# The dashboard (development needs Node; the corvus binary does not).
ui-deps:
	cd ui && npm ci --no-audit --no-fund

ui: ui-deps
	cd ui && npm run build

test: ui test-rust test-ui

test-rust:
	$(CARGO) test --workspace

test-ui:
	cd ui && npm test && npm run scan

# Dashboard bundle in WebKit and Chromium against a throwaway corvus serve; app checks that need a GUI session.
test-e2e: ui
	$(CARGO) build -p corvus-cli
	cd ui && npm run e2e
	CORVUS_GUI_TESTS=1 $(CARGO) test -p corvus-desktop

lint-ui: ui-deps
	cd ui && npm run lint && npm run check

# The corvus binary must build without Node or the desktop app's toolkit.
check-cli-node-free:
	@if $(CARGO) tree -p corvus-cli -e normal --prefix none --format '{p}' | awk '{print $$1}' | grep -qE '^(tauri|tauri-build|wry|tao|corvus-desktop)$$'; then \
	  echo "corvus-cli must not depend on the desktop app"; exit 1; fi
	@echo "cli without desktop deps: ok"

# Regenerate the dashboard's TypeScript API types from the Rust structs (ts-rs).
types:
	rm -rf ui/src/lib/types
	$(CARGO) test -q -p corvus-core --lib export_bindings > /dev/null

# Generated types must match the Rust structs (fails on drift; run `make types` and commit).
check-types:
	@tmp=$$(mktemp -d); TS_RS_EXPORT_DIR=$$tmp $(CARGO) test -q -p corvus-core --lib export_bindings > /dev/null; \
	if ! diff -r $$tmp ui/src/lib/types > /dev/null; then echo "ui/src/lib/types is stale: run make types"; diff -r $$tmp ui/src/lib/types | head -20; rm -rf $$tmp; exit 1; fi; \
	rm -rf $$tmp; echo "generated types: ok"

lint: check-purity check-rusqlite check-sql-clock check-seam check-types check-cli-node-free lint-ui
	$(CARGO) fmt --all -- --check
	@test -d ui/dist || $(MAKE) ui
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

# corvus-domain is pure: no IO crates among its dependencies, no IO in its source.
check-purity:
	@deps="$$($(CARGO) tree -p corvus-domain -e normal --prefix none --format '{p}' | awk '{print $$1}')"; \
	bad="$$(echo "$$deps" | grep -E '^(rusqlite|libsqlite3-sys|tokio|reqwest|hyper|axum|mio|socket2)$$' || true)"; \
	if [ -n "$$bad" ]; then echo "corvus-domain must be pure; it depends on: $$bad"; exit 1; fi
	@if grep -rnE 'std::(fs|net|process|env)|SystemTime|Instant::now|tokio|rusqlite|now_v7|new_v4' crates/corvus-domain/src; then \
	  echo "corvus-domain must be pure (see matches above)"; exit 1; fi
	@echo "domain purity: ok"

# Only corvus-core depends on rusqlite, and only corvus-core::db opens connections.
check-rusqlite:
	@for c in $(NON_CORE_CRATES); do \
	  if $(CARGO) tree -p $$c -e normal,dev,build --depth 1 --prefix none --format '{p}' | awk 'NR>1{print $$1}' | grep -qx rusqlite; then \
	    echo "$$c must not depend on rusqlite"; exit 1; fi; \
	done
	@if grep -rnE 'Connection::open|open_with_flags|pragma_update|PRAGMA (journal_mode|foreign_keys|synchronous|busy_timeout|trusted_schema|query_only) *=' crates --include=*.rs \
	  | grep -v '^crates/corvus-core/src/db/'; then \
	  echo "only corvus-core::db may open connections or set pragmas"; exit 1; fi
	@echo "rusqlite confinement: ok"

# D0007: no SQL clock functions; time is bound as a parameter from the application clock.
check-sql-clock:
	@if grep -rniE "current_(timestamp|date|time)|unixepoch *\(|(datetime|date|time|julianday|strftime) *\([^)]*'now'" \
	  crates/corvus-core/migrations crates --include=*.sql --include=*.rs; then \
	  echo "SQL clock functions are forbidden (D0007); bind :now from the Clock instead"; exit 1; fi
	@echo "sql clock: ok"

# The unauthenticated inner router is used only by corvus-http (wrapped by serve()) and the desktop app (in memory).
check-seam:
	@if grep -rn 'data_router' crates --include=*.rs | grep -vE '^crates/corvus-(http|desktop)/'; then \
	  echo "data_router() may only be used inside corvus-http and corvus-desktop"; exit 1; fi
	@echo "http seam: ok"

run:
	$(CARGO) run -p corvus-cli -- health

# ---------------------------------------------------------------------------------------------------------------
# Release artifacts. Official builds run these targets in GitHub Actions (.github/workflows/release.yml, D0026);
# `make dist` builds the same set on a Mac, with podman for the Linux builds and smoke tests.
# Signing: set CORVUS_SIGN_IDENTITY to a keychain identity name; nothing secret lives in the repo.
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
COMMIT := $(shell git rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
DIST := target/dist
MACOS_TARGETS := aarch64-apple-darwin x86_64-apple-darwin
LINUX_ARCHES := arm64 amd64
LINUX_IMAGE ?= docker.io/library/rust:1-slim
SMOKE_IMAGE ?= docker.io/library/debian:stable-slim
triple_arm64 := aarch64-unknown-linux-musl
triple_amd64 := x86_64-unknown-linux-musl

.PHONY: dist dist-clean dist-macos dist-app

# Every artifact plus SHA256SUMS in target/dist.
dist: dist-clean dist-macos dist-app $(addprefix dist-linux-,$(LINUX_ARCHES)) $(addprefix smoke-linux-,$(LINUX_ARCHES))
	cd $(DIST) && shasum -a 256 *.tar.gz *.zip > SHA256SUMS && cat SHA256SUMS

dist-clean:
	rm -rf $(DIST) && mkdir -p $(DIST)


dist-macos:
	@for t in $(MACOS_TARGETS); do rustup target add $$t > /dev/null; done
	@for t in $(MACOS_TARGETS); do \
	  $(CARGO) build --release --locked -p corvus-cli --target $$t || exit 1; \
	  mkdir -p $(DIST)/stage/$$t && cp target/$$t/release/corvus README.md $(DIST)/stage/$$t/; \
	done
	@for t in $(MACOS_TARGETS); do b=$(DIST)/stage/$$t/corvus; \
	  if [ -n "$$CORVUS_SIGN_IDENTITY" ]; then \
	    codesign --force --options runtime --sign "$$CORVUS_SIGN_IDENTITY" $$b && codesign --verify --strict $$b && echo "signed: $$b" || exit 1; \
	  else echo "unsigned: $$b (set CORVUS_SIGN_IDENTITY to sign)"; fi; \
	done
	@for t in $(MACOS_TARGETS); do \
	  COPYFILE_DISABLE=1 tar -C $(DIST)/stage/$$t -czf $(DIST)/corvus-$(VERSION)-$$t.tar.gz corvus README.md; \
	done
	@arch -x86_64 $(DIST)/stage/x86_64-apple-darwin/corvus --version
	@$(DIST)/stage/aarch64-apple-darwin/corvus --version

# The desktop app for this Mac's architecture, zipped with its signature intact.
dist-app: bundle
	ditto -c -k --keepParent target/release/bundle/macos/Corvus.app $(DIST)/Corvus-$(VERSION)-macos-$(shell uname -m).app.zip

# Static musl binaries, cross-compiled in a throwaway container of this machine's architecture (clang + rust-lld
# + Debian's musl headers for the target). Emulating the foreign architecture instead crashed rustc.
BUILD_ARCH := $(if $(filter arm64 aarch64,$(shell uname -m)),arm64,amd64)
gnuarch_arm64 := aarch64
gnuarch_amd64 := x86_64
dist-linux-%:
	mkdir -p target/linux-$* $(DIST)/stage/$(triple_$*)
	podman run --rm --platform linux/$(BUILD_ARCH) -v $(CURDIR):/src -v corvus-cargo-$(BUILD_ARCH):/usr/local/cargo/registry \
	  -e CARGO_TARGET_DIR=/src/target/linux-$* -e CORVUS_GIT_COMMIT=$(COMMIT) -w /src $(LINUX_IMAGE) sh -c '\
	  set -e; dpkg --add-architecture $* && apt-get update -qq && apt-get install -y -qq clang lld musl-dev:$* > /dev/null; \
	  rustup target add $(triple_$*) > /dev/null; \
	  export CC_$(subst -,_,$(triple_$*))=clang \
	    CFLAGS_$(subst -,_,$(triple_$*))="--target=$(gnuarch_$*)-linux-musl -isystem /usr/include/$(gnuarch_$*)-linux-musl" \
	    CARGO_TARGET_$(shell echo $(triple_$*) | tr a-z- A-Z_)_LINKER=rust-lld; \
	  cargo build --release --locked -p corvus-cli --target $(triple_$*)'
	cp target/linux-$*/$(triple_$*)/release/corvus README.md $(DIST)/stage/$(triple_$*)/
	COPYFILE_DISABLE=1 tar -C $(DIST)/stage/$(triple_$*) -czf $(DIST)/corvus-$(VERSION)-$(triple_$*).tar.gz corvus README.md

# The same static musl binary built natively on a Linux machine of that architecture (the release workflow's
# runners; needs musl-tools). No container, no cross-compiling.
native-linux-%:
	rustup target add $(triple_$*) > /dev/null
	mkdir -p $(DIST)/stage/$(triple_$*)
	CORVUS_GIT_COMMIT=$(COMMIT) $(CARGO) build --release --locked -p corvus-cli --target $(triple_$*)
	cp target/$(triple_$*)/release/corvus README.md $(DIST)/stage/$(triple_$*)/
	tar -C $(DIST)/stage/$(triple_$*) -czf $(DIST)/corvus-$(VERSION)-$(triple_$*).tar.gz corvus README.md

# Clean-machine smoke test: only the binary in a stock image; health, cross-process claim race, MCP, hook.
define SMOKE
set -e; export HOME=/tmp/h CORVUS_STATE_DIR=/tmp/h/.corvus; mkdir -p $$HOME; C=/c/corvus; cd /tmp
$$C --version
ldd $$C 2>&1 | grep -qE "not a dynamic executable|statically linked"; echo "static: ok"
$$C health 2>/dev/null | grep -q '"ok":true'; echo "health: ok"
$$C task create --title race > /dev/null 2>&1
for i in 1 2 3 4 5 6 7 8; do $$C session register --agent racer$$i 2>/dev/null | sed -n 's/.*"id":"\([^"]*\)".*/\1/p' > s$$i; done
for i in 1 2 3 4 5 6 7 8; do ( $$C task claim --session $$(cat s$$i) > c$$i 2>/dev/null ) & done; wait
w=$$(cat c1 c2 c3 c4 c5 c6 c7 c8 | grep -c '"status":"claimed"' || true); echo "claim race winners: $$w"; [ "$$w" = 1 ]
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' '{"jsonrpc":"2.0","method":"notifications/initialized"}' '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | CORVUS_AGENT_NAME=smoke $$C mcp 2>/dev/null > mcp.out
n=$$(grep '"id":2' mcp.out | grep -o '"name":"[a-z_]*"' | wc -l); echo "mcp tools: $$n"; [ "$$n" -ge 20 ]
$$C tell racer1 "smoke hello" > /dev/null 2>&1
echo '{"hook_event_name":"UserPromptSubmit","session_id":"smoke","prompt":"hi"}' | CORVUS_AGENT_NAME=racer1 $$C hook | grep -q "smoke hello"; echo "hook: ok"
echo "smoke: all ok"
endef
export SMOKE

smoke-linux-%:
	podman run --rm --platform linux/$* -v $(CURDIR)/$(DIST)/stage/$(triple_$*):/c:ro -e SMOKE $(SMOKE_IMAGE) sh -c "$$SMOKE"
