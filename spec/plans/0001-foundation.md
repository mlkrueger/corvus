# Plan 0001 — Foundation: workspace, config, embedded database, migrations

- **Status:** Done
- **Depends on:** —
- **Spec:** [system/01](../system/01-architecture.md) · [system/02](../system/02-data-model.md) · [system/10](../system/10-config-ops.md) · brief §2 (P1, P4, P9, P10), §3, §6

## Goal

A building Cargo workspace whose `corvus` binary creates, migrates and opens its own SQLite database with zero
configuration, with typed configuration, JSON logging, the pure domain types, and a test harness that gives
every test a fresh database file.

## Scope

**In:** workspace + crate skeletons; `corvus-domain` types and validators; `Config`, `Clock`, env-file loader;
`corvus-core::db` (the one opener, pragmas, connection set, migrator, backup); all migrations (including
work-source tables and hints); `corvus migrate`, `corvus health`, `corvus admin backup`; `Makefile` with `test`
and `lint`; `.env.example`.

**Out:** service behaviour (0002+); the HTTP server (0004); stack commands (0007); packaging (0011).

## Work breakdown

- [x] 1. Workspace: `Cargo.toml`, crates per [system/01](../system/01-architecture.md#workspace-layout) as
      skeletons, `rust-toolchain.toml`, `rustfmt`/`clippy` config, `.gitignore`, `.env.example`.
- [x] 2. `corvus-domain`: `AgentName`, ids, `TaskStatus`, `MessageType`, `EndReason`, `ArtifactRef` (path and
      commit validation), `Scope`, `EventKind`, `CorvusError`; property tests for every validator.
- [x] 3. `corvus-core::config` (typed env parsing, defaults, redacting `Debug`), env-file loader (`0600`
      check), state-directory creation with permissions, `Clock` (`SystemClock`, `ManualClock`), `tracing`
      JSON-to-stderr setup.
- [x] 4. `corvus-core::db`: `open()` with verified pragmas; writer + readers; read-only open for the hook;
      `write(|tx| …)` helper that issues `BEGIN IMMEDIATE`, checks the schema version, handles busy/retry;
      `read(|tx| …)` helper.
- [x] 5. Migrator: embedded files, `schema_migrations`, SHA-256 checksums, single write transaction,
      `user_version`, refusal on a newer database, pre-migration backup via `VACUUM INTO`.
- [x] 6. Migrations `0001`–`0006` exactly as [system/02](../system/02-data-model.md) and
      [system/07](../system/07-work-sources.md#storage) define, including append-only and hint triggers.
- [x] 7. `corvus migrate`, `corvus health`, `corvus admin backup`.
- [x] 8. Test harness: temp-dir database per test, fixtures for agents/sessions/tasks, a helper that spawns
      the built `corvus` binary against a given database.
- [x] 9. `make lint`: fmt, clippy `-D warnings`, domain-purity check, "only `corvus-core::db` depends on
      rusqlite" check, grep for SQL clock functions ([D0007](../decisions/0007-time-source.md)).

## Verification

- [x] **Zero config:** with an empty environment and an empty temp `HOME`, `corvus health` creates the state
      directory (`0700`) and database (`0600`), migrates, and prints the schema version.
- [x] Migrations apply to an empty database; a second run is a no-op.
- [x] Editing an applied migration (test alters an embedded copy) fails with a checksum error and applies nothing.
- [x] **Locked:** N processes started together on a fresh database all succeed and each migration is applied once.
- [x] A database with a higher `user_version` than the binary is refused with a clear message.
- [x] A pre-migration backup file is written when migrating a non-empty database, and it opens.
- [x] Pragmas are in effect on every connection (`foreign_keys`, `journal_mode`, `synchronous`, `busy_timeout`);
      a foreign-key violation is actually rejected.
- [x] `UPDATE`, `DELETE` with a `WHERE`, and bare `DELETE` on `messages`, `digests`, `events` each fail.
- [x] Constraint tests: second active claim per task / per session rejected; reply across threads rejected;
      artifact with `..` or absolute path rejected; empty title/body rejected; over-long body rejected; every
      malformed agent name rejected and every valid shape accepted (property test against the domain validator,
      so the SQL `GLOB` check and the Rust check agree).
- [x] Inserting a message writes `inbox:<recipient>` / `inbox:*` to `hints`; inserting or updating a task writes `tasks`.
- [x] A write helper used from a read path, or a read upgraded to a write, is impossible by API shape (compile-time).
- [x] Config: each variable's default, each invalid value's error; secrets absent from `Debug` output.
- [x] Env file: existing env wins; group/world-readable file refused.
- [x] `corvus admin backup` produces a consistent copy while another process is writing.

## Exit criteria

- [x] `make lint` and `make test` pass (on the development machine; see *Unverified* for the clean-machine part).
- [x] `corvus health` prints `{"ok": true, "schema_version": 6}` on a machine that has never run Corvus.

## Open questions

- None.

## Unverified / deferred

- **Clean machine.** `make lint` / `make test` were run on the development machine (macOS, Rust 1.98.1), not on a
  machine with *only* Rust and a C compiler. Crate download needs network. The clean-machine smoke test belongs to
  [plan 0011](0011-distribution.md) (podman).
- **`make test` scope.** It runs `cargo test --workspace` only; the `ui` unit tests and bundle scan from
  [system/09](../system/09-testing.md#commands) join it when `ui/` exists (plan 0006). `make lint` likewise lacks
  the `ui` lint and generated-types checks until then.
- **Health output formatting.** On a pipe, `corvus health` prints compact JSON `{"ok":true,"schema_version":6}`;
  on a terminal, the same object pretty-printed ([system/04](../system/04-interfaces.md#cli-corvus)).
- **"Started together".** The multi-process migration test spawns 16 `corvus migrate` processes back to back
  (3 rounds) rather than releasing them from a barrier file; they overlap in practice, and every round asserts
  each migration was applied exactly once.
- **Busy exhaustion** (`busy_timeout` then two retries then `Busy`) is implemented in `Database::write` and the
  migrator but has no dedicated test yet; plan 0002's race tests assert that no operation returns `Busy`.

## Notes from implementation

- `system/02` corrected: SQLite cannot name `rowid` in an index; the claim-scan index is
  `(priority DESC, created_at)`, to which SQLite appends rowid implicitly.
- `system/02` corrected: `agents.name` also requires `instr(name, char(0)) = 0`. `length()` and `GLOB` stop at a NUL,
  so `'a' || char(0) || '/../x'` passed the sketched check. Found by the SQL-vs-domain property test.
- Id generation (UUIDv7) lives in `corvus-core::ids`, keeping `corvus-domain` free of clock and randomness.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0013 | SQLite, compiled into the binary and used in WAL mode, is the single store — the only option that is truly embedded yet keeps invariants in the store. | [0013](../decisions/0013-storage-sqlite.md) |
| D0014 | rusqlite with synchronous services and a small in-house migrator; tokio, axum, rmcp and clap at the edges. | [0014](../decisions/0014-rust-stack-rusqlite.md) |
| D0015 | One `corvus` binary; every process opens the database itself and migrates on open. Shapes the crate layout and the opener. | [0015](../decisions/0015-process-topology-single-binary.md) |
| D0007 | Time comes from an injectable application clock, never SQL clock functions, so expiry and presence are testable without sleeping. | [0007](../decisions/0007-time-source.md) |
| D0010 | Append-only tables are protected by raising triggers on UPDATE and DELETE. | [0010](../decisions/0010-append-only-enforcement.md) |
