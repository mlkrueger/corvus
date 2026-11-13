# D0012 — Comment tracking by order key + tie set

- **Status:** Accepted
- **Date:** 2026-11-13
- **Plans:** [0008](../plans/0008-work-sources-and-linear.md), [0009](../plans/0009-jira-source.md)
- **Spec:** [system/07](../system/07-work-sources.md#storage) · brief §4.9.3–4

## Problem

The engine must deliver each tracker comment exactly once per link. The brief specifies a "numeric comment
high-water mark": Jira comment ids are increasing integers, so "id above the mark" identifies new comments.
Linear comment ids are UUIDs with no order. The shared engine ([D0011](0011-work-source-abstraction.md)) needs
one mechanism that is faithful to the brief for Jira and correct for Linear.

## Options

1. **Numeric high-water mark only**, with Linear using the comment's creation time as the number.
2. **A seen-set table** — one row per processed comment id per link.
3. **High-water mark on an adapter-supplied order key, plus the ids already processed at exactly that key.**

## Trade-offs

### Numeric mark, Linear by timestamp
- **For:** identical to the brief; one integer per link.
- **Against:** two Linear comments created in the same millisecond straddling a poll would lose the second one
  — rare, but a silent loss of a user's comment.

### Seen-set table
- **For:** exact for any id scheme; no ordering assumption at all.
- **Against:** departs from the brief's stated high-water mark; a row per comment forever; "new" requires an
  anti-join instead of a comparison.

### Order key + tie set
- **For:** for Jira it reduces exactly to the brief's numeric mark (ids never tie, the tie set stays a single
  id); for Linear it is exact even with identical timestamps; constant storage per link.
- **Against:** slightly more subtle than either pure approach; relies on the order key being non-decreasing for
  comments created later (true for Jira ids and for creation timestamps).

## Decision

**Option 3.** Each adapter gives every comment an opaque `id` and an integer `order_key` (Jira: the numeric id;
Linear: `createdAt` in epoch milliseconds). A link stores `comment_hwm` and `hwm_ids`. A comment is new when
`order_key > comment_hwm`, or `order_key = comment_hwm` and its id is not in `hwm_ids`. After processing, the
mark moves to the largest key seen and `hwm_ids` becomes the ids at that key.

## Consequences

- A comment back-dated by the tracker to before the mark would be skipped. Neither tracker does this for new
  comments; edits to old comments are deliberately not re-delivered.
- Comments from ignored accounts advance the mark without being delivered.
- The comparison is unit-tested as a pure function in `corvus-domain`, including the tie case.

## Discussion

- 2026-11-13 — Initial record.
- 2026-11-13 — Storage moved to SQLite (D0013); `hwm_ids` is stored as a JSON array in a TEXT column. No change to the mechanism.
