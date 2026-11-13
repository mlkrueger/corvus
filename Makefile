# Development conveniences only; nothing here is needed to build or run Corvus.
.PHONY: ui ui-deps types check-types test test-rust test-ui test-e2e lint lint-ui check-cli-node-free fmt check-purity check-rusqlite check-sql-clock check-seam run

CARGO ?= cargo
NON_CORE_CRATES := corvus-domain corvus-jira corvus-linear corvus-http corvus-mcp corvus-cli corvus-desktop

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
