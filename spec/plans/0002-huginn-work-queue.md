# Plan 0002 — Huginn: sessions, tasks, claims, leases, cancellation

- **Status:** Ready
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

- [ ] 1. Sessions: `register_session` (agent upsert, name generation and reservation), `heartbeat`, the
      "touch session" primitive, `end_session`, `list_sessions`.
- [ ] 2. Presence: pure `presence(...)` in `corvus-domain`; table-driven tests over every boundary.
- [ ] 3. `create_task`: validation, idempotency, dependencies, thread, artifacts, event.
- [ ] 4. `claim_task` with the reap step; `claimant_of`.
- [ ] 5. `renew_lease`, `release_task`, `complete_task` (ownership, replays).
- [ ] 6. `expire_leases` as a standalone operation.
- [ ] 7. `cancel_task` with cascade and claimant notification.
- [ ] 8. `end_stale_sessions` (auto-end).
- [ ] 9. `list_tasks`, `get_task` with derived `blocked` / `blocked_by_cancelled` / holder; `append_to_task`.
- [ ] 10. Minimal CLI commands + the multi-process race harness.

## Verification

All against real database files with `ManualClock`
([technique](../system/09-testing.md#concurrency-test-technique)).

- [ ] **Claim race (threads):** 16+ connections claim one task → exactly one claim row, the rest get nothing;
      no errors, no `Busy`.
- [ ] **Claim race (processes):** the same with spawned `corvus` processes on one database file.
- [ ] **Claim fan-out:** N claimers, N tasks → N distinct tasks claimed.
- [ ] **Queueing is bounded:** under the stress run, the longest write transaction and the longest wait for the
      write lock stay under a stated ceiling.
- [ ] Claim order: priority desc, then oldest. Pool filter. Recipient routing. Idempotent per session.
- [ ] Dependencies: blocked until all deps `done`; cancelled dep keeps it blocked and sets `blocked_by_cancelled`.
- [ ] Lease: expires exactly at the boundary; reaped by `claim_task`; task reopens; history row has `expired`.
- [ ] Renew by holder extends; by another session → conflict; after expiry+reap → conflict.
- [ ] Release reopens; complete is final; replays by the same session are no-ops; other sessions → conflict.
- [ ] Cancel: open and claimed tasks; done → conflict; idempotent; claimant gets a `status` message and then a
      conflict on renew/complete; cascade cancels the whole tree and skips done dependents.
- [ ] **Cancel versus claim:** cancel and claim of the same task fired together, many iterations, threads and
      processes → every outcome is one of the two serial orders (claimed-then-cancelled with the claim ended
      `cancelled`, or cancelled-and-never-claimed); never a live claim on a cancelled task.
- [ ] **Mixed stress:** claim / renew / release / complete / cancel / expire / end-session in parallel for many
      iterations → no hang, no `Busy`, invariants hold (≤ 1 active claim per task and per session; task status
      consistent with claims).
- [ ] The unique indexes are a real backstop: a test bypasses the service and attempts a second active claim
      by direct SQL → rejected.
- [ ] Idempotency: `create_task` and `register_session` replays, sequential and concurrent → one row, one event.
- [ ] Unknown dependency / recipient → not found. Ended or unknown session → `SessionNotActive`.
- [ ] `end_session` releases the held task. Generated names are unique under concurrent registration.
- [ ] **Auto-end vs heartbeat:** fired together, many iterations → either the heartbeat landed first and the
      session survives, or auto-end landed first and the heartbeat gets `SessionNotActive`; never an ended
      session with a later `last_seen_at`, never a surviving session whose claim was released. Threshold 0 disables.
- [ ] Every mutation above wrote exactly one audit event with no body text.

## Exit criteria

- [ ] `make lint` and `make test` pass; the race tests pass 50 consecutive runs locally.
- [ ] Every operation in the lock-protocol section of system/03 is a single `BEGIN IMMEDIATE` transaction in code.

## Open questions

- None.

## Unverified / deferred

—

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0017 | Every mutation is one short `BEGIN IMMEDIATE` transaction, so operations are serial across processes and cannot deadlock; partial unique indexes back it up. Claimers queue briefly rather than run in parallel — a stated departure from the brief. | [0017](../decisions/0017-single-writer-concurrency.md) |
| D0006 | Idempotency is a `(creator, idempotency_key)` partial unique index per table with `ON CONFLICT DO NOTHING`; replays return the original. | [0006](../decisions/0006-idempotency-keys.md) |
| D0007 | An injectable clock supplies `now`, so lease and presence boundaries are tested exactly and without sleeps. | [0007](../decisions/0007-time-source.md) |
