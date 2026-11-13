# AGENTS.md — how to work in this repo

Corvus is a local-first coordination service for AI coding agents. It ships as **one self-contained Rust
binary** (embedded SQLite, no containers, no required services) plus a self-contained **Tauri 2** desktop app.
This file is the rulebook for every agent (and human) working here. Read it before changing anything.

## 1. Where things live

| Path | What it is | Rule |
|---|---|---|
| [`spec/README.md`](spec/README.md) | Map of the spec tree. **Start here.** | Keep its indexes current. |
| [`spec/brief.md`](spec/brief.md) | The original requirements (WHAT). | **Frozen. Never edit.** |
| [`spec/system/`](spec/system/README.md) | The system spec (HOW). Guides everything. | Source of truth for design. Update it *before* code that changes design. |
| [`spec/plans/`](spec/plans/README.md) | One plan per feature / unit of work. | Every non-trivial change has a plan. |
| [`spec/decisions/`](spec/decisions/README.md) | One record per significant decision. | Every significant choice has a record. |

Precedence when documents disagree: **owner direction > brief > system spec > plan**. Owner direction that
overrides the brief is recorded in a decision and listed in
[`spec/system/11-defaults.md`](spec/system/11-defaults.md#deliberate-departures-from-the-briefs-wording).
A plan may not contradict the system spec; fix the spec first (with a decision record if the change is a real
choice), then the plan.

## 2. The workflow

1. **Find the spec.** Locate the system-spec section and plan covering your work via `spec/README.md`.
2. **No plan? Write one.** Copy [`spec/plans/_template.md`](spec/plans/_template.md) to
   `spec/plans/NNNN-short-slug.md` (next free number) and add it to the index in `spec/plans/README.md`.
3. **Hit a real choice? Record a decision** (see §4) before building on it.
4. **Build in the plan's order**, ticking its checkboxes as steps land. Keep the build green at every step.
5. **Verify for real.** Run the tests; run the binary and exercise the command, endpoint or UI you changed. Do
   not claim behaviour you have not exercised. Record what you could not verify in the plan.
6. **Close the loop.** Update the plan's status, and the system spec if reality diverged from it.

## 3. Plans

- File name: `spec/plans/NNNN-short-slug.md`, four-digit number, never reused, never renumbered.
- Use the template. Required sections: Status, Goal, Scope, Spec references, Work breakdown, Verification,
  Exit criteria, Open questions, **Decisions**.
- Status is one of `Draft | Ready | In progress | Done | Abandoned`.
- A plan says *what will be built and in what order*. Design that outlives the plan belongs in `spec/system/`.
- **The plan's last section is always `## Decisions`**: a table with, for each decision the plan relies on or
  produced, its ID, a one- or two-sentence summary, and a link to the full record. Never restate the full
  reasoning in the plan; the record is the home for detail and discussion.
- Plans link only to **current** decisions. When a decision is superseded, update the plans that used it.

## 4. Decisions

Write a decision record when a choice is (a) hard to reverse, (b) had real alternatives, or (c) someone will
later ask "why is it like this?". Small defaults where the brief is silent go in
[`spec/system/11-defaults.md`](spec/system/11-defaults.md) instead.

- File name: `spec/decisions/NNNN-short-slug.md`; cite as `D0007`. Numbers are never reused.
- Use [`spec/decisions/_template.md`](spec/decisions/_template.md). Every record has, in this order:
  1. **Problem** — what must be decided and why it matters.
  2. **Options** — every option seriously considered.
  3. **Trade-offs** — for each option, what it buys and what it costs.
  4. **Decision** — what was chosen **and why**.
  Plus: Consequences, and a dated Discussion log for anything that comes up later.
- **Links are bidirectional and mandatory.** The record lists the plans that use it (`Plans:` header); each of
  those plans lists the record in its `## Decisions` table. Add the record to `spec/decisions/README.md`.
- Status is `Proposed | Accepted | Superseded by DNNNN | Rejected`.
- **Accepted records are immutable** except for: status changes, link maintenance, and appended Discussion entries.
- **To change a decision, supersede it**: write a new record with a `Supersedes:` header; set the old record's
  status to `Superseded by DNNNN` and add a Discussion entry saying why; move every plan and system-spec link
  to the new record; update the index. The old record stays as history, with its original `Plans:` line.

## 5. Engineering rules (enforced)

These come from the brief's non-negotiable properties (`P1`..`P10`), owner direction and the system spec.
Violations are bugs.

- **Self-contained.** The `corvus` binary must run on a clean machine with nothing else installed: no
  containers, no database server, no required daemon, no shell scripts, no Node at build or run time for users.
  Do not add a runtime dependency on an external service or tool. ([system/01](spec/system/01-architecture.md))
- **Layering.** `corvus-domain` is pure (no IO, no async runtime, no SQL). Services in `corvus-core` own every
  transaction. Transports (`corvus-http`, `corvus-mcp`, `corvus-cli`, the desktop IPC command) only parse, call
  a service, and serialise. No business logic in handlers.
- **One database opener.** Only `corvus-core::db` opens connections or sets pragmas. No other crate uses `rusqlite`.
- **Write transactions** always start with `BEGIN IMMEDIATE`, are never upgraded from a read, and never contain
  network IO, file IO, subprocesses or waiting. ([system/03](spec/system/03-services.md#lock-protocol))
- **Migrations are one-way and immutable.** Never edit an applied migration; add a new one.
- **Append-only tables** (`messages`, `digests`, `events`) are never updated or deleted — the store rejects it.
- **Every state change writes an audit event**, in the same transaction, with no message bodies.
- **Secrets** come only from the environment. Never in code, logs, the database, test fixtures, or URLs sent to
  servers. Never log message bodies or tokens.
- **Untrusted text stays text.** In `ui/`: no `{@html}`, no `innerHTML`/`outerHTML`/`insertAdjacentHTML`,
  no inline scripts or `style=""` attributes. Text from Jira or Linear is labelled untrusted wherever it is copied.
- **New agent tools are adapters.** Support for a tool is an adapter file run by the one adapter runner — never
  a special case in the supervisor. No adapter may carry message text into a prompt.
  ([system/06](spec/system/06-delivery-helpers.md#agent-adapters))
- **Notifications are hints.** Subscribe before reading; always re-read state after a wake-up.
- **Background loops never die.** Log the failure, isolate the step, continue.
- **Tests that touch concurrency or atomicity run against real SQLite files** (not mocks, not `:memory:`), and
  cross-process guarantees are tested cross-process.
- **Every SQL statement is covered by a test** — there is no compile-time query checking.

## 6. Tooling

- Rust stable, edition 2024. `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` must pass before work is called done. The test suite needs no services.
- Frontend (`ui/`): Svelte 5 **runes** (`$state`, `$derived`, `$effect`), TypeScript, Vite. No Svelte 4 patterns.
  The dashboard ships only inside the desktop app: no browser UI, no `/ui` route, no listener in the app.
  Generated TypeScript types are committed; regenerate them when the Rust API types change.
- If a container is ever needed for development (for example a clean-machine smoke test): **`podman`**, never
  `docker`. Python, if ever needed for scripts: **`uv`**, never `pip`. Neither may become a product dependency.
- GitHub operations via `gh`.
- Never run `sudo` or other password-prompting commands; print the command for the user to run.
- Commit, push, or take destructive actions **only when the user asks**.

## 7. Writing style for spec documents

- Lead with the answer. Short sections, tables over prose, stable headings (other files link to them).
- Use relative links between spec files so they work in any viewer.
- Cite requirements as `brief §4.2` or `P2`, decisions as `D0005`, plans as `plan 0003`.
- One fact lives in one place; everywhere else links to it.
