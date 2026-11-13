# D0006 — Per-table `(creator, idempotency_key)` unique index

- **Status:** Accepted
- **Date:** 2026-11-13
- **Plans:** [0002](../plans/0002-huginn-work-queue.md), [0003](../plans/0003-muninn-hermod.md)
- **Spec:** [system/02](../system/02-data-model.md) · [system/03](../system/03-services.md#other-task-operations) · P3

## Problem

Every create operation accepts an optional `idempotency_key` scoped to the creator; a replay must return the
original result and create nothing (P3). Agents retry freely — after timeouts, crashes, or simply by
regenerating a tool call — so this must hold under concurrency, including two identical requests in flight at
once. Creates here are: sessions, tasks, threads, messages, digests, decisions.

## Options

1. **A nullable `idempotency_key` column on each creatable table** with a partial unique index on
   `(creator, idempotency_key)`, written with `INSERT … ON CONFLICT DO NOTHING`.
2. **One central `idempotency_keys` table** mapping `(agent, operation, key)` to the created resource.
3. **Central table that also stores the serialised response** and a request fingerprint, rejecting a key reused
   with a different payload.

## Trade-offs

### Per-table column
- **For:** the store enforces it with an ordinary constraint; one statement decides create-vs-replay, and a
  concurrent duplicate simply waits on the index and then sees the conflict; no extra table; works for
  append-only tables because it only inserts.
- **Against:** the pattern is repeated on six tables; keys are namespaced per table, so the same key on a task
  and on a message are unrelated (acceptable); the replay returns the resource's *current* state, not a stored
  copy of the first response.

### Central table
- **For:** one mechanism, one code path.
- **Against:** the resource id must be known before the resource exists (awkward for identity-generated message
  ids) or the insert order becomes delicate; two rows per create; nothing gained over a constraint.

### Central table with stored response + fingerprint
- **For:** exact replay of the first response; detects key misuse.
- **Against:** stores response bodies (including message text) a second time; a fingerprint mismatch turns a
  harmless agent retry with regenerated wording into an error — the opposite of what retry-happy agents need.

## Decision

**Option 1.** Each creatable table has `idempotency_key` and a partial unique index with its creator column
(`creator`, `sender`, `author`, `decided_by`, `agent_name`). Services insert with `ON CONFLICT DO NOTHING`; when
no row comes back they load and return the original and skip all side effects (no second thread, no second
event). A replay with a different payload returns the original unchanged: the key is the identity.

## Consequences

- Threads additionally keep their separate global `key` (brief §3); that is a naming feature, not this mechanism.
- Replays return current state. For append-only resources that equals the original; for a task it may show a
  later status, which is the more useful answer.
- The work-source engine relies on this for "idempotent per ticket + stamp".

## Discussion

- 2026-11-13 — Initial record.
- 2026-11-13 — Storage moved to SQLite (D0013). The mechanism is unchanged: SQLite supports partial unique indexes and `ON CONFLICT … DO NOTHING`. Concurrent duplicates now serialise on the write lock instead of waiting on the index.
