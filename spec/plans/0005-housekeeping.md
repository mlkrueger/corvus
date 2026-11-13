# Plan 0005 — Housekeeping loop and background leader

- **Status:** Done
- **Depends on:** [0004](0004-transports.md)
- **Spec:** [system/10](../system/10-config-ops.md#housekeeping) · [system/10](../system/10-config-ops.md#background-leader) · brief §4.8 (P8)

## Goal

While a long-lived Corvus process runs (`corvus serve` now, the desktop app later), expired leases are reaped,
silent sessions are ended and hints are pruned about every 60 seconds — in exactly one process at a time — and
no failure in that loop can stop the process.

## Scope

**In:** `housekeeping_pass()`, the loop task, per-step isolation (errors and panics), the background-leader
file lock and hand-over, shutdown integration, the admin routes/commands wired to the same functions.

**Out:** the operations themselves (built in 0002); source polling, which reuses the leader (0008).

## Work breakdown

- [x] 1. `housekeeping_pass(services) -> PassReport` with named steps, each in its own error boundary.
- [x] 2. `BackgroundLeader`: exclusive OS file lock on `run/leader.lock`, `leader.json`, 30 s retry for
      non-holders, release on shutdown.
- [x] 3. Loop task: runs only while leader; first pass after 5 s, then every 60 s; skips a tick if the previous
      pass is still running.
- [x] 4. Wire `POST /api/admin/expire-leases`, `/end-stale-sessions` and the CLI admin commands.
- [x] 5. Shutdown: the loop stops promptly; the WAL is checkpointed.

## Verification

- [x] A pass with an expired lease, a stale session and excess hints handles all three and reports counts.
- [x] **Isolation:** inject a failing `expire_leases` (error, then panic) → later steps still run; the report
      records the failure; a log line names the step.
- [x] **Never fatal:** with the database made unavailable mid-run (file lock held by a test, then released),
      the loop logs, keeps ticking and recovers; the HTTP server keeps answering.
- [x] **One leader:** two processes started together → exactly one runs passes; kill it → the other takes over
      within the retry interval; `leader.json` reflects the holder.
- [x] `CORVUS_AUTO_END_SECONDS=0` skips the session step.
- [x] Paused-time test: passes fire at 5 s, 65 s, 125 s.
- [x] With **no** long-lived process running, an expired lease is still reaped by the next `claim_task`.
- [x] By hand: claim a task with a 10-second lease, stop heartbeating, watch it reopen in the logs and events.

## Exit criteria

- [x] `make lint` and `make test` pass.

## Open questions

- None.

## Unverified / deferred

- **"Never fatal" with the database unavailable** was tested as: another connection holds the write lock for
  1.5 s while the loop runs (passes wait it out and keep ticking afterwards), plus failing and panicking steps on
  every pass (the loop keeps ticking). The loop's HTTP-server-keeps-answering part was not tested in the same run;
  the loop and the HTTP server are separate tasks, and the HTTP suite runs independently.
- **One leader across processes** was checked by hand (two `corvus serve` processes on one state directory:
  `leader.json` named the first; after `kill -9` the second took over 23 s later, and its first pass reaped an
  expired 10 s lease; SIGTERM exit 0). The automated leader tests run two loops in one process (the lock is an OS
  file lock per open file, so they contend the same way).
- `corvus serve` takes the leader lock before it binds the port; if the bind fails it exits and the OS releases
  the lock at once.
- Hint pruning runs in housekeeping only, not also on open (D0016 mentions both).

## Notes from implementation

- `housekeeping_pass_with(services, steps)` takes the step list, so tests inject failing and panicking steps;
  `LoopTimings` carries the timings and the steps.
- A dropped shutdown sender stops the loop (found while wiring `corvus serve`: an early exit would otherwise have
  spun the loop).
- The workspace minimum Rust version is now 1.89 (`File::try_lock` for the leader lock; `rmcp` needs 1.88).

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0017 | The sweep's steps are ordinary short write transactions, serialised with live traffic by SQLite's write lock, so the sweep can never deadlock with or corrupt agent operations. | [0017](../decisions/0017-single-writer-concurrency.md) |
| D0007 | Expiry and staleness are judged against the injectable clock, so the sweep is tested with exact times and no sleeps. | [0007](../decisions/0007-time-source.md) |
