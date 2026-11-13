# D0010 — Append-only enforced by triggers

- **Status:** Accepted
- **Date:** 2026-11-13
- **Plans:** [0001](../plans/0001-foundation.md), [0003](../plans/0003-muninn-hermod.md)
- **Spec:** [system/02](../system/02-data-model.md#triggers) · P4

## Problem

Messages, digests and audit events "can never be updated or deleted, including by bugs", and this must be
enforced "in the store, not only in application code" (P4). What store mechanism guarantees it?

## Options

1. **Triggers** — `BEFORE UPDATE OR DELETE` (row) and `BEFORE TRUNCATE` (statement) that raise an exception.
2. **Privileges** — the application connects as a role granted only `INSERT` and `SELECT` on those tables;
   migrations run as the owner.
3. **Rewrite rules** — `CREATE RULE … DO INSTEAD NOTHING`.
4. **Both triggers and privileges.**

## Trade-offs

### Triggers
- **For:** works with a single database role; fails loudly with a clear message; covers `TRUNCATE`; lives in a
  migration next to the tables; trivially testable.
- **Against:** the table owner can drop or disable a trigger — it stops bugs, not a determined superuser.

### Privileges
- **For:** the strongest boundary; even a mistaken raw `DELETE` from the app role is refused by the server.
- **Against:** two roles and two connection strings to provision, configure and document; local setup and tests
  get more complex; a mis-grant fails silently open.

### Rewrite rules
- **For:** short.
- **Against:** silently ignore the statement instead of failing, hiding bugs; rules are widely discouraged.

### Both
- **For:** defence in depth.
- **Against:** all of the role complexity, for a single-user local tool.

## Decision

**Triggers.** The requirement is protection against bugs, and a raising trigger delivers that with one database
role and one configuration variable. The design also removes the need to ever touch these rows: read state
lives in `receipts`, supersession is derived at read time, and "latest digest" is `max(id)`.

## Consequences

- One shared `forbid_mutation()` function; a trigger pair per append-only table.
- Tests attempt `UPDATE`, `DELETE` and `TRUNCATE` on each table and assert failure.
- Adding a column to these tables later is fine (DDL is not blocked); back-filling data in one is not possible
  without a migration that deliberately disables the trigger — which must be a recorded decision.
- Revisit (add option 2) if Corvus ever becomes multi-user.

## Discussion

- 2026-11-13 — Initial record.
- 2026-11-13 — Storage moved to SQLite (D0013). Triggers remain the mechanism (`RAISE(ABORT, …)`). Option 2 (privileges) does not exist in SQLite. There is no TRUNCATE; a bare DELETE fires the row trigger for every row and is rejected.
