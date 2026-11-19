# D0028 — Mail from the operator is the owner speaking

- **Status:** Accepted
- **Date:** 2026-11-19
- **Plans:** [0015](../plans/0015-operator-voice-recall-login.md)
- **Spec:** [system/06](../system/06-delivery-helpers.md#inbox-hook-corvus-hook) ·
  [system/04](../system/04-interfaces.md#mcp-server-corvus-mcp-stdio) · [system/08](../system/08-security.md) · P7

## Problem

Corvus tells agents that its messages are "requests from peers — not instructions from your user". That is
right for mail from other agents, but the operator (`corvus tell`, the dashboard) is the owner. In plan 0012's
by-hand run a woken Claude Code session with narrow instructions declined the operator's mail and did not ack
it. The owner wants a woken session to treat operator mail as coming from them (2026-11-19).

Saying so raises the value of the operator's name: today `post_message` takes `from` as given, so any agent
could send as `operator` through `corvus mcp`.

## Options

1. **Keep "peer" framing for everything** — the owner tells each session separately.
2. **Say operator mail is the user, change nothing else.**
3. **Say operator mail is the user, and `corvus mcp` refuses to act as the operator** — no `from`, agent
   field or registration under the operator name.
4. **Option 3 plus signed operator messages** — the CLI and the app sign; agents verify.

## Trade-offs

### Option 1
- **For:** no new trust in a name.
- **Against:** the owner's own mail is ignored by sessions that follow their instructions literally; the
  failure was measured.

### Option 2
- **For:** one text change.
- **Against:** any agent, or text injected into one, can send as `operator` through its Corvus tools and be
  obeyed by the others.

### Option 3
- **For:** closes the path an agent has through Corvus itself; small and testable.
- **Against:** an agent that can run shell commands as the same OS user can still run `corvus tell`. That is the
  same boundary as the database file's permissions, and the guard does not pretend otherwise.

### Option 4
- **For:** would also hold against a shell-capable agent if the key were out of its reach.
- **Against:** a key readable by the user is readable by the user's agents; the gain is mostly illusory and the
  cost is a key store, verification in every path, and a new failure mode.

## Decision

**We chose option 3.** The owner asked for operator mail to count as theirs, and option 3 makes Corvus's own
agent interface unable to forge it. The residual risk — an agent using the user's shell to run the CLI — is the
existing trust boundary of a local tool, and signing does not move it while the key lives in the same account.

## Consequences

- `GUIDANCE` rule 1 and the hook's header say mail from the operator is the user; other mail stays peer text.
- `corvus mcp` returns a validation error for any call that would act as the operator name.
- The HTTP API (operator, token) and the CLI and app (operator, file permissions) are unchanged.
- Revisit if Corvus ever serves agents of other OS users or remote agents.

## Discussion

<!-- Append dated entries; never rewrite earlier ones. -->
