# D0025 — The operator can reassign an agent's unread mail; it is a transfer, not an acknowledgement

- **Status:** Accepted
- **Date:** 2026-11-16
- **Plans:** [0012](../plans/0012-session-identity-and-wake.md)
- **Spec:** [system/03](../system/03-services.md#muninn-messaging) · [system/02](../system/02-data-model.md#receipts) · brief §4.2, §4.7

## Problem

An agent is one tool conversation ([D0023](0023-agent-identity-follows-conversation.md)). When a conversation
is abandoned for good, its unread mail can never be read: the owner does not want a mailbox that "just seems
dead". The brief says only the agent itself acknowledges a message, and `messages` is append-only, so the mail
can be neither marked read by someone else nor re-addressed.

## Options

1. **Warn and flag only** — the mail stays unread for ever.
2. **Take-over by name** — a new session registers under the old agent's name and inherits the inbox.
3. **Reassign** — the operator moves an agent's unread mail to another agent.

## Trade-offs

### Warn and flag
- **For:** nothing to build; no rule bent.
- **Against:** the dead mailbox stays dead; unread counts never clear.

### Take-over by name
- **For:** already works — an unlinked conversation that registers with an existing name is linked to that agent.
- **Against:** only before the new session has registered. One that already has its own name cannot switch (a
  conversation is one agent for good).

### Reassign
- **For:** works for any two agents at any time; the new agent sees the mail as its own unread mail and is
  woken by it; the old inbox empties, so counts are honest.
- **Against:** someone other than the recipient changes a message's read state; the original and the copy are
  two rows; an agent could be handed mail it has no context for.

## Decision

**We chose reassign (owner direction, 2026-11-16), keeping take-over by name as it is.** A mailbox must not
look dead, and take-over alone does not cover a session that already has a name.

- **Operator only**: `corvus mail reassign <from> <to>` and a dashboard action. It is not an MCP tool; agents
  cannot move each other's mail.
- **What moves**: every unread message addressed to `from` (direct mail). Broadcasts are not moved — `to`
  already has its own copy of each.
- **How**: for each one, in one transaction: post a copy to `to` (same thread, same type, sender unchanged,
  body unchanged, `reassigned_from` = the original id) and write a receipt on the original for `from` with
  `kind = 'reassigned'`, `reassigned_to = to`. The copy is ordinary unread mail for `to`.
- **Not an acknowledgement**: the receipt kind is distinct. Views show "reassigned to `to`", never "read by
  `from`". The brief's rule stands for acknowledgements: only the agent acks.
- Reassigning is idempotent (an already-reassigned message is skipped) and audited (`message.reassigned`, no
  bodies).

## Consequences

- `receipts` gains `kind` and `reassigned_to`; `messages` gains `reassigned_from`. Unread stays "no receipt",
  so nothing else changes.
- A reply to a reassigned question goes to the original sender as usual (the sender is unchanged).
- If the old conversation is later resumed, its agent finds an empty inbox; the history shows where the mail
  went.
- Revisit if agents need to hand mail to each other, or if reassignment should be offered automatically when a
  new session starts in an abandoned agent's directory.

## Discussion

- 2026-11-16 — Created from the owner's direction: "I don't want a mailbox to just seem dead. We should have
  the ability to re-assign messages to a new agent"; copy-and-mark confirmed.
