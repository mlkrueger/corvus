# D0023 — An agent's identity follows its tool conversation, so hooks need no per-project name

- **Status:** Accepted
- **Date:** 2026-11-16
- **Plans:** [0012](../plans/0012-session-identity-and-wake.md)
- **Spec:** [system/06](../system/06-delivery-helpers.md#inbox-hook-corvus-hook) · [system/04](../system/04-interfaces.md) · brief §3 Session, §4.7

## Problem

The inbox hook learns which agent it serves from `CORVUS_AGENT_NAME` written into the hook command. That works
for one agent per project. The owner runs many sessions on one host (ten at once is normal), wants every one of
them reachable, addresses mail to a **specific session**, and wants a restarted session to get its inbox back
rather than leave it dead.

A hook installed once at user level runs in every session, so a fixed name would make them all the same agent.
Something else has to say who each session is.

## Options

1. **Per-project name** (today) — the name is written into each project's hook command.
2. **Name from the working directory** — one user-level hook; the folder maps to the agent.
3. **Name from the tool's conversation id** — Corvus records which conversation belongs to which agent when the
   session registers; hooks look the conversation up.

## Trade-offs

### Per-project name
- **For:** nothing new to build.
- **Against:** a setup step per project; two sessions in one project are the same agent; a session in a project
  without the hook is unreachable.

### Working directory
- **For:** one setup for everything; a new session in the same folder inherits the inbox.
- **Against:** two sessions in one folder collide; every session in any folder becomes an agent, wanted or not;
  it addresses a place, and the owner addresses a session.

### Conversation id
- **For:** exact — one conversation, one agent, one inbox. A resumed conversation keeps its id, so it gets the
  same agent back. A session that never registered has no link, so user-level hooks ignore it. The id is
  available without the model's help (measured 2026-11-16, Claude Code 2.1.325: `CLAUDE_CODE_SESSION_ID` is in
  the MCP server's environment; `session_id` is in every hook's input; the hook and the MCP server are children
  of the same `claude` process).
- **Against:** a new table and a lookup. `/clear` issues a new id inside the same process, so the link has to
  be carried over. A brand-new conversation is a new agent, so the old agent's unread mail needs a way out.
  The id sources are per tool and not all documented.

## Decision

**We chose the conversation id (owner direction, 2026-11-16).** The owner addresses specific sessions and runs
several per folder, which rules out the other two. The rules:

- **Link.** When a session registers through `corvus mcp`, Corvus records `(tool, conversation id) → agent`.
  The id comes from the tool, not the model: the MCP server's environment (Claude Code) or the MCP request
  metadata (OpenCode, Codex — to be confirmed in [plan 0012](../plans/0012-session-identity-and-wake.md)).
- **Resume.** Registering with a known conversation attaches to that conversation's agent: same name, same
  inbox, a new Corvus session.
- **Clear.** A cleared session keeps its agent: the link is carried from the old id to the new one.
- **Fork** and **new conversation** are new agents.
- **Hooks resolve by conversation.** With no `CORVUS_AGENT_NAME`, the hook looks its `session_id` up. No link →
  it exits silently, so a user-level hook leaves unconnected sessions alone. An explicit `CORVUS_AGENT_NAME`
  still wins (today's per-project setup keeps working).
- **Stranded mail is visible.** An agent with unread mail and no live session is flagged, and sending to it
  warns that it is offline.

## Consequences

- New append-light table for the links (a migration), written in the same transaction as the registration, with
  an audit event.
- `register_session` gains an optional conversation reference; `corvus mcp` fills it in. Other transports are
  unchanged.
- Two processes on one conversation (a resume while the original still runs) are two Corvus sessions of one
  agent. That is already legal; both see the same inbox.
- The carry-over on `/clear` needs one write from the hook path, which is otherwise read-only — decided in plan
  0012 step 1 and recorded in system/06.
- Taking over a stranded agent's name from a new conversation ("adoption") is not built yet; revisit when
  stranded mail turns out to be common.
- The id sources are tool internals. If a tool stops exposing its id, that tool falls back to the explicit name.

## Discussion

- 2026-11-16 — Created from the owner's choice of "option 3" and "specific session, but resolve a restart".
  Measurements behind it (four interactive Claude Code sessions): `--resume` and `--continue` keep the id;
  `/clear` fires `SessionEnd reason=clear` with the old id then `SessionStart source=clear` with a new one, in
  the same process, without restarting the MCP server.
- 2026-11-16 — Accepted by the owner.
