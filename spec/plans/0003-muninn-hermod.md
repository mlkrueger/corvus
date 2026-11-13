# Plan 0003 — Muninn + Hermod: messaging, memory, export, wake-ups

- **Status:** Ready
- **Depends on:** [0002](0002-huginn-work-queue.md)
- **Spec:** [system/03](../system/03-services.md#muninn-messaging) · brief §4.3, §4.4, §4.5, §4.6 (P3, P4, P5)

## Goal

Agents can talk and remember: threads, messages, receipts, direct threads, blocking inbox waits that wake across
processes, digests, decisions, the bootstrap context, the guidance text, stats/overview, and the git export.

## Scope

**In:** `Muninn` service; `Hermod` listener and `wait_for_inbox`; `bootstrap`, `connect`; `stats`, `overview`,
`health`; `export_state_to_git`; the `GUIDANCE` constant.

**Out:** transports (0004); the dashboard that consumes `overview` (0006).

## Work breakdown

- [ ] 1. Threads: `create_thread` (keyed idempotency, reserved `direct:` prefix).
- [ ] 2. `post_message` (validation, reply-in-thread, artifacts, idempotency, session ownership, event).
- [ ] 3. `send_direct` (pair key), `read_inbox`, `ack_message`, `list_messages` with read/unread-by.
- [ ] 4. Hermod: per-process `data_version` watcher on a dedicated read connection (active only while there
      are subscribers), hint-cursor reads, topic fan-out, wake-all when the cursor was pruned past, subscription API.
- [ ] 5. `wait_for_inbox`: subscribe → heartbeat → read → wait/re-check loop; timeout clamp.
- [ ] 6. Digests and decisions: write, latest, list, supersession.
- [ ] 7. `GUIDANCE` constant; `bootstrap`; `connect`.
- [ ] 8. `stats`, `overview`, `health`.
- [ ] 9. `export_state_to_git`: snapshot read in one read transaction, then (outside any transaction) atomic
      file writes, stale-file removal, optional commit.

## Verification

- [ ] Unread semantics: addressed, broadcast, own messages excluded, acked excluded; `after_id` pages but an
      older unread message that commits later is still returned by a plain read.
- [ ] Ack: idempotent, returns `newly_acked`; own message → conflict; not addressed → conflict.
- [ ] Unresolved question: resolved only by an `answer` with `in_reply_to`; a `note` reply does not resolve.
- [ ] Reply to a message in another thread → validation error (and the store rejects a forced insert).
- [ ] Direct threads: A→B and B→A share one thread; A→C is a different one; concurrent first messages create one thread.
- [ ] `list_messages`: newest first, paging, filters, `read_by`/`unread_by`; calling it never creates a receipt.
- [ ] Idempotency replays for thread, message, digest, decision — sequential and concurrent.
- [ ] Append-only: service has no update/delete path; direct SQL attempts fail (re-asserted here for digests/messages).
- [ ] **Wake-up (same process):** waiter blocks; a message posted on another connection wakes it within the
      detection interval.
- [ ] **Wake-up (cross process):** a waiter in one `corvus` process is woken by a message posted by another
      `corvus` process on the same database file.
- [ ] **Targeting:** a hint for another agent does not cause this waiter to re-read; `inbox:*` does.
- [ ] **Timeout:** no message → returns empty at the deadline (paused tokio time); timeout > 120 is clamped.
- [ ] **Subscribe-before-read:** a message committed between subscribe and read is returned immediately.
- [ ] **Lost hint:** with hint detection disabled, and separately with the `hints` table emptied under the
      waiter's cursor, a waiter still returns via the periodic re-check / wake-all.
- [ ] No waiter → no polling (the watcher is idle when there are no subscribers).
- [ ] Coalesced / duplicate hints cause no duplicate results. Waiting bumps `last_seen_at`.
- [ ] Digests: latest per scope key; unknown ref → not found. Decisions: superseded hidden by default.
- [ ] Bootstrap contains exactly the six parts; `connect` = session + bootstrap + guidance.
- [ ] Export: all files present with expected content; paths returned are repo-relative; absolute or `..`
      subdir rejected; no repo configured → validation error; `commit=true` commits only exported paths in a
      temp git repo with unrelated dirty files left untouched; stale `inbox/` file removed.
- [ ] Stats and overview numbers match a seeded scenario, including `blocked_by_cancelled` and presence counts.

## Exit criteria

- [ ] `make lint` and `make test` pass.
- [ ] Two `corvus`-linked test processes exchange a message with a blocking wait end to end.

## Open questions

- None.

## Unverified / deferred

—

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0016 | Triggers write targeted hints to a small table in the same transaction; each process detects commits with `data_version` and wakes matching waiters. Waiters subscribe first and re-check every 10 s, so a lost hint costs latency, never correctness. | [0016](../decisions/0016-wakeups-hints-table.md) |
| D0006 | Per-table idempotency keys; replays return the original and skip side effects. | [0006](../decisions/0006-idempotency-keys.md) |
| D0010 | Messages, digests and events are append-only by trigger; read state lives in `receipts` so nothing ever needs updating. | [0010](../decisions/0010-append-only-enforcement.md) |
