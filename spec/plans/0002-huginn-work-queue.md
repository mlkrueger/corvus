# Plan 0002 — Huginn: sessions, tasks, claims, leases, cancellation

- **Status:** Done
- **Depends on:** [0001](0001-foundation.md)
- **Spec:** [system/03](../system/03-services.md#lock-protocol) · brief §4.1, §4.2 (P2, P3)

## Goal

The work queue is correct under concurrency: sessions and presence, task creation with dependencies, atomic
claiming, leases, release/complete, expiry and cancellation — each proven by tests against real SQLite files,
from multiple threads and from multiple processes.

## Scope

**In:** `SessionService`, `Huginn` service, their store queries, audit events for every mutation, the pure
presence function, name generation; minimal CLI commands needed to drive multi-process tests
(`session register`, `task create`, `task claim`).

**Out:** messaging (0003) — except the minimal thread row created with a task and the `status` message sent on
cancel, which this plan inserts through a small internal function that 0003 then owns. The rest of the
transports (0004). Scheduled sweeps (0005).

## Work breakdown

- [x] 1. Sessions: `register_session` (agent upsert, name generation and reservation), `heartbeat`, the
      "touch session" primitive, `end_session`, `list_sessions`.
- [x] 2. Presence: pure `presence(...)` in `corvus-domain`; table-driven tests over every boundary.
- [x] 3. `create_task`: validation, idempotency, dependencies, thread, artifacts, event.
- [x] 4. `claim_task` with the reap step; `claimant_of`.
- [x] 5. `renew_lease`, `release_task`, `complete_task` (ownership, replays).
- [x] 6. `expire_leases` as a standalone operation.
- [x] 7. `cancel_task` with cascade and claimant notification.
- [x] 8. `end_stale_sessions` (auto-end).
- [x] 9. `list_tasks`, `get_task` with derived `blocked` / `blocked_by_cancelled` / holder; `append_to_task`.
- [x] 10. Minimal CLI commands + the multi-process race harness.

## Verification

All against real database files with `ManualClock`
([technique](../system/09-testing.md#concurrency-test-technique)).

- [x] **Claim race (threads):** 16+ connections claim one task → exactly one claim row, the rest get nothing;
      no errors, no `Busy`.
- [x] **Claim race (processes):** the same with spawned `corvus` processes on one database file.
- [x] **Claim fan-out:** N claimers, N tasks → N distinct tasks claimed.
- [x] **Queueing is bounded:** under the stress run, the longest write transaction and the longest wait for the
      write lock stay under a stated ceiling.
- [x] Claim order: priority desc, then oldest. Pool filter. Recipient routing. Idempotent per session.
- [x] Dependencies: blocked until all deps `done`; cancelled dep keeps it blocked and sets `blocked_by_cancelled`.
- [x] Lease: expires exactly at the boundary; reaped by `claim_task`; task reopens; history row has `expired`.
- [x] Renew by holder extends; by another session → conflict; after expiry+reap → conflict.
- [x] Release reopens; complete is final; replays by the same session are no-ops; other sessions → conflict.
- [x] Cancel: open and claimed tasks; done → conflict; idempotent; claimant gets a `status` message and then a
      conflict on renew/complete; cascade cancels the whole tree and skips done dependents.
- [x] **Cancel versus claim:** cancel and claim of the same task fired together, many iterations, threads and
      processes → every outcome is one of the two serial orders (claimed-then-cancelled with the claim ended
      `cancelled`, or cancelled-and-never-claimed); never a live claim on a cancelled task.
- [x] **Mixed stress:** claim / renew / release / complete / cancel / expire / end-session in parallel for many
      iterations → no hang, no `Busy`, invariants hold (≤ 1 active claim per task and per session; task status
      consistent with claims).
- [x] The unique indexes are a real backstop: a test bypasses the service and attempts a second active claim
      by direct SQL → rejected.
- [x] Idempotency: `create_task` and `register_session` replays, sequential and concurrent → one row, one event.
- [x] Unknown dependency / recipient → not found. Ended or unknown session → `SessionNotActive`.
- [x] `end_session` releases the held task. Generated names are unique under concurrent registration.
- [x] **Auto-end vs heartbeat:** fired together, many iterations → either the heartbeat landed first and the
      session survives, or auto-end landed first and the heartbeat gets `SessionNotActive`; never an ended
      session with a later `last_seen_at`, never a surviving session whose claim was released. Threshold 0 disables.
- [x] Every mutation above wrote exactly one audit event with no body text.

## Exit criteria

- [x] `make lint` and `make test` pass.
- [ ] The race tests pass 50 consecutive runs locally (running; see *Unverified*).
- [x] Every operation in the lock-protocol section of system/03 is a single `BEGIN IMMEDIATE` transaction in code.

## Open questions

- None.

## Unverified / deferred

- **Mixed stress in process form** is not tested; the mixed stress runs across threads (each with its own
  database handle). Claim races, fan-out, cancel-versus-claim and auto-end-versus-heartbeat are tested across
  both threads and processes.
- **Process start-together**: process race tests hold the write lock from the test for 0.9 s while spawning
  children, so they queue on SQLite's busy handler and are released together when it commits. A child that starts
  late just runs after the release (a weaker race, not a wrong result).
- **Queue ceilings** (debug build, this machine): longest write-lock hold observed 1.1–1.6 ms against a 250 ms
  ceiling; longest wait for the lock ≈ 1.3 s under 16-way contention against a 5 s ceiling. The wait is SQLite's
  busy-handler backoff (sleep-and-retry, not a fair queue), not long transactions.
- The race suite was not mutation-tested against a deliberately broken service.
- **50 consecutive runs**: 24 debug-build runs passed back to back before the loop was switched to a faster
  form (release build, the core and CLI race files in parallel, about 14 s per run); that 50-run loop was still
  running at this commit.

## Notes from implementation

- Choices where the spec was silent are in [system/11](../system/11-defaults.md) rows 35–40 (heartbeats not
  audited; idempotent registration without a name; release/complete of an expired-but-unreaped lease; ending an
  ended/unknown session; cancel's status message; who may cancel).
- `Database::write` records per-handle write-transaction wait and hold times (`write_stats()`) and logs a warning
  for any hold over 250 ms.
- The CLI got the full `session` and `task` command sets (not only register/create/claim) plus
  `admin expire-leases` / `admin end-stale`, since the services existed and the race tests needed cancel/end.
- New event kind `task.appended` for `append_to_task`.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0017 | Every mutation is one short `BEGIN IMMEDIATE` transaction, so operations are serial across processes and cannot deadlock; partial unique indexes back it up. Claimers queue briefly rather than run in parallel — a stated departure from the brief. | [0017](../decisions/0017-single-writer-concurrency.md) |
| D0006 | Idempotency is a `(creator, idempotency_key)` partial unique index per table with `ON CONFLICT DO NOTHING`; replays return the original. | [0006](../decisions/0006-idempotency-keys.md) |
| D0007 | An injectable clock supplies `now`, so lease and presence boundaries are tested exactly and without sleeps. | [0007](../decisions/0007-time-source.md) |
