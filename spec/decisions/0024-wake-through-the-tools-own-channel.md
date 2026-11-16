# D0024 — Idle sessions are woken through each tool's own channel; Corvus does not own the terminal

- **Status:** Accepted
- **Date:** 2026-11-16
- **Plans:** [0012](../plans/0012-session-identity-and-wake.md)
- **Spec:** [system/06](../system/06-delivery-helpers.md) · brief §4.7

## Problem

A session sitting idle at its prompt does not see new mail until someone types. The owner needs to wake it from
outside, for interactive sessions started the normal way, and asked whether that means Corvus must own the
session through a multiplexer.

The built-in `codex` adapter wakes by headless resume, which collides with a session that is open in a window
([plan 0007](../plans/0007-delivery-helpers.md) open question). A headless resume of a live Claude Code session
interleaves two writers in one transcript.

## Options

1. **Own the terminal** — `corvus run <agent> -- <tool>` holds the PTY; wake by typing the fixed note.
2. **External multiplexer** — tmux/zellij `send-keys`.
3. **Headless resume** — today's adapter mechanism.
4. **Each tool's own channel** — whatever the tool offers for starting a turn in a running session.

## Trade-offs

### Own the terminal
- **For:** works for any tool; also gives a live view and a true attach.
- **Against:** Corvus becomes the launcher for every session; a per-session holder process and a socket that
  carries terminal contents and accepts input; redraw, resize and input-collision problems. Sessions started
  normally stay unwakeable.

### External multiplexer
- **For:** almost free (`corvus watch --exec tmux send-keys …` works today).
- **Against:** a required external tool breaks the self-contained rule, so it can only ever be optional.

### Headless resume
- **For:** already built.
- **Against:** wrong for a session that is open: a second writer on the same conversation.

### The tool's own channel
- **For:** no ownership; sessions are started the normal way. All three target tools have one:

  | Tool | Channel | Evidence |
  |---|---|---|
  | Claude Code | a background hook (`asyncRewake`) that waits for mail and exits 2 | **tested** 2026-11-16 on 2.1.325: woke an idle interactive session twice; re-arms after each turn; picked up when added to a running session |
  | OpenCode 2.x | the shared background service: `opencode run --session <id> "<note>"` (or `POST /api/session/{id}/prompt`) | docs and the installed 2.3.0 CLI; **not yet run** against a live session |
  | Codex ≥ 0.157 | the shared daemon: `codex queue --thread <id> --message "<note>"` | docs and a repo test; **not run** (Codex is not installed) |

- **Against:** one mechanism per tool, each resting on that tool's internals (two of the Claude Code hook
  options next to `asyncRewake` are marked internal). No live view or attach comes with it.

## Decision

**We chose each tool's own channel.** It meets the actual requirement — wake a normally started session — with
no ownership, and it is the only option already shown to work for Claude Code. Owning the terminal is not
needed for waking any of the three tools, so it is not built now.

- **Claude Code** is woken from inside: a second hook entry runs a waiter (`corvus hook wait`) in the
  background. It blocks on the agent's inbox, costs no model tokens, and exits 2 with the formatted mail when
  something new arrives. It shares the inbox hook's shown-set, so a message wakes a session once.
- **OpenCode and Codex** are woken from outside by the existing wake supervisor and an adapter. The adapter's
  `{conversation}` comes from the conversation link ([D0023](0023-agent-identity-follows-conversation.md)), so
  there is no manual `corvus agent bind`.
- The rule from [D0021](0021-agent-adapters.md) stands: adapters pass only the fixed note. The Claude Code
  waiter shows message text exactly as the inbox hook already does.

## Consequences

- Each built-in wake path is confirmed against a real session of that tool before it ships; an unconfirmed one
  stays documented as unverified, not enabled.
- The built-in `codex` adapter's headless resume is replaced once `codex queue` is confirmed on an installed
  Codex.
- A session is not wakeable until its tool has run the hook or registered once (for Claude Code: after the
  session-start or first stop event).
- **Stepping in** is left to the tools: OpenCode and Codex share sessions through their own services
  (`opencode --session <id>`, `codex resume <id>`); Claude Code can only be resumed after it exits. A Corvus
  terminal holder (`corvus run`) remains the way to get a live attach for Claude Code — revisit if that is
  wanted, with its own decision (the socket would carry data, unlike [D0022](0022-desktop-single-instance-local-socket.md)).
- Seeing what a session is doing (an activity feed from hook events) is separate work, not decided here.

## Discussion

- 2026-11-16 — Created after the owner asked how to "step into" and wake running sessions. Tests and research
  are summarised in [plan 0012](../plans/0012-session-identity-and-wake.md#unverified--deferred).
- 2026-11-16 — Accepted by the owner.
- 2026-11-16 — Both remaining channels confirmed on real sessions with `corvus serve` as the leader: OpenCode
  2.3.0 (`opencode run --session`) and Codex 0.169.0 (`codex queue --thread … --resume`); each idle session
  answered a question in-thread with nobody typing. The built-in `codex` adapter now uses `codex queue`.
  `codex exec resume` against an open window fails with "already has an active writer" rather than interleaving.
