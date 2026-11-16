# Plan 0012 — Session identity and waking idle sessions

- **Status:** In progress
- **Depends on:** [0007](0007-delivery-helpers.md)
- **Spec:** [system/06](../system/06-delivery-helpers.md) · [system/04](../system/04-interfaces.md) · [system/02](../system/02-data-model.md) · brief §3 Session, §4.7

## Goal

Any Claude Code session on the machine that has connected to Corvus is its own agent, is woken when mail
arrives while it sits idle, and gets the same agent and inbox back when its conversation is resumed — set up
once at user level, with no per-project names. OpenCode and Codex sessions get the same identity and are woken
through their own services where that is confirmed to work.

## Scope

**In:** conversation links (table, service, `register_session`, `corvus mcp`); hook identity by conversation;
carry-over on `/clear`; the waiter (`corvus hook wait`); user-level hook setup docs and the app's snippet;
stranded-mail visibility; an `opencode` adapter and the `codex` adapter change. All three tools are in scope
(owner, 2026-11-16); a wake path still ships only once it is confirmed on a real session of that tool.

**Out:** owning a session's terminal (`corvus run`), live attach, an in-app terminal, remote access
([D0024](../decisions/0024-wake-through-the-tools-own-channel.md)); an activity feed of what each session is
doing (the next plan, 0013 — owner agreed the order 2026-11-16); taking over a stranded agent's name from a new conversation.

## Work breakdown

- [ ] 1. **Spikes (gate — results recorded below before step 2).**
  - [x] a. Claude Code waiter edge cases: wake while the user is typing; wake while a turn is running; the
        longest a background hook may run and what happens at its timeout; whether a `SessionStart` entry arms
        before the first turn; a headless `claude -p` run must still exit promptly with the waiter registered.
  - [x] b. Waiter lifetime: it exits when its `claude` process is gone (killed, crashed, terminal closed).
  - [x] c. `/clear` carry-over: mechanism chosen (a note at `SessionEnd reason=clear`, one write at
        `SessionStart source=clear`; [system/06](../system/06-delivery-helpers.md#session-events)); proved on a
        real session in the by-hand checks once step 6 is built.
  - [x] d. OpenCode 2.3 live: the session id reaches `corvus mcp` (`_meta["ai.opencode/sessionID"]`);
        `opencode run --session <id> "<note>"` starts a turn in an idle open session and the open window shows it.
  - [ ] e. *(pinned by the owner 2026-11-16 — Codex 0.169.0 is installed but not signed in; come back to it)*
        Codex: `codex queue --thread <id> --message "<note>"` on an idle
        open session; where the id reaches `corvus mcp`.
- [x] 2. **Spec first.** system/02 (table), /03 (service), /04 (`register_session`, MCP), /06 (hook identity,
      waiter, adapters bound from links), /08 (threats), /11 (defaults); accept or amend D0023 and D0024.
- [x] 3. Migration `0007_conversations`: `(tool, conversation_id) → agent_name`, working directory, timestamps.
      Link and carry-over written in one transaction with an audit event.
- [x] 4. `register_session` takes an optional conversation reference: known → attach to its agent; unknown →
      register as today and record the link. `corvus mcp` fills it from the environment or request metadata.
- [ ] 5. `corvus hook` resolves its agent by conversation when `CORVUS_AGENT_NAME` is absent; no link → silent
      exit 0. Explicit name still wins.
- [ ] 6. `/clear` carry-over, by the mechanism chosen in 1c.
- [ ] 7. `corvus hook wait`: one waiter per (agent, conversation) under a file lock (a second exits 0); blocks
      on the inbox for mail not in the shown-set; exit 2 with the formatted messages on stderr and the shown-set
      updated; exit 0 when its session is gone; never fails in any other way.
- [ ] 8. Setup: user-level settings snippet in [docs/delivery-helpers.md](../../docs/delivery-helpers.md) and
      the app's "Connect an agent" view; README quick start.
- [ ] 9. Stranded mail: `corvus tell` warns when the recipient has no live session; the dashboard flags agents
      with unread mail and no live session and shows each session's working directory.
- [ ] 10. Built-in `opencode` adapter, bound from the conversation link (enabled only if 1d passed).
- [ ] 11. *(pinned with 1e)* `codex` adapter wakes with `codex queue` (enabled only if 1e passed; otherwise stays
      as is and is reported).

## Verification

- [ ] Links: register with an unknown, a known and no conversation; resume attaches to the same agent; two live
      sessions on one conversation; link + event are atomic; every new SQL statement has a test.
- [ ] Hook identity: linked conversation → that agent's mail; unlinked → empty output, exit 0, database not
      opened for writing; explicit name wins; hostile conversation ids cannot escape the state directory.
- [ ] Carry-over: after a clear, the new id resolves to the same agent; an unrelated session's clear does not.
- [ ] Waiter (real binary, temp database): blocks with no mail; exits 2 with the message when another process
      posts; a message already shown does not wake again; a second waiter exits 0; exits 0 when the parent
      process dies; every failure path (no database, schema mismatch, busy, bad stdin) exits 0 silently.
- [ ] No wake loop: unread-but-shown mail across ten stop events produces zero wakes.
- [ ] By hand, real Claude Code, user-level hooks: two sessions in one folder are two agents; `corvus tell` to
      one wakes only that one; quit and `claude --resume` → same agent, unread mail delivered; `/clear` keeps
      the agent; a session that never connected is untouched.
- [ ] By hand, OpenCode (and Codex if installed): tell → the idle session starts a turn and reads its inbox.
- [ ] `corvus tell` to an agent with no live session prints the offline warning; the dashboard flags it.

## Exit criteria

- [ ] `make lint`, `make test` and `make test-e2e` pass.
- [ ] The by-hand Claude Code checks are recorded below with a date.
- [ ] Each built-in wake path is either confirmed on a real session or listed as unverified and not enabled.

## Open questions

- **Resolved in step 2** (recorded in [defaults](../system/11-defaults.md) rows 60–65): the carry-over
  mechanism (row 63), the waiter time limit (row 61). **Recommended defaults the owner has not confirmed:**
  direct mail only wakes (row 62), `corvus up` starts watchers (row 64), unreachable agents get a warning and a
  flag, no take-over (row 65).
- **Resolved in step 4:** Claude Code 2.1.325 sends no `initialize` request to a stdio server on protocol
  `2026-07-28`; it names itself (`claude-code`) in every request's `_meta["io.modelcontextprotocol/clientInfo"]`.
  OpenCode names itself at `initialize`. The server reads both.
- **`opencode run` and the adapter timeout**: the command returns when the turn ends, which can be longer than
  300 s. Does killing it cancel the turn? Check in step 10.
- **An install command** (`corvus setup claude` editing `~/.claude/settings.json`) versus documented snippets
  only. Editing the user's settings file is the owner's call.
- **`--fork-session`** is assumed to issue a new id (a new agent); untested.

## Unverified / deferred

Measured before this plan was written (2026-11-16, Claude Code 2.1.325, interactive sessions in a
pseudo-terminal, project-level settings):

- A `Stop` hook with `asyncRewake: true` that waits for a trigger and exits 2 **woke an idle session** (twice),
  and re-armed after the woken turn.
- The same hook **added to the settings file of a running session** took effect at that session's next stop
  (the file existed at startup; a user-level file was not tried).
- `claude --resume <id>` and `--continue` keep the conversation id; `/clear` issues a new one in the same
  process (`SessionEnd reason=clear`, then `SessionStart source=clear`); the MCP server is not restarted by it.
- `CLAUDE_CODE_SESSION_ID` is set in the MCP server's environment at start (stale after a clear); hooks get
  `session_id` on stdin and `CLAUDE_PID` in the environment; hooks and the MCP server share the parent process.
- The waiter in those tests **outlived a killed session** (the reason for 1b).

### Spike results (step 1, 2026-11-16)

Claude Code 2.1.325, interactive sessions in a pseudo-terminal, a prototype shell waiter:

| Question | Result | Consequence |
|---|---|---|
| Wake while the user is typing | The session woke and answered; the half-typed text stayed in the input box untouched. | No collision handling needed. |
| Wake while a turn is running | The running turn finished, then the wake started a new turn straight away. | Nothing to do; wakes queue behind the turn. |
| How a wake arrives | As a prompt: `UserPromptSubmit` fires, and the `Stop` after it has `stop_hook_active=true`. | The inbox hook's prompt path runs on a wake too; the shown-set must make the two agree. |
| Time limit | Default **600 s**: the waiter got `SIGTERM` at 10 minutes and the session stayed asleep afterwards. A 20 s limit behaved the same way. | The hook entry must set a long `timeout`; the waiter treats `SIGTERM` as a normal exit. |
| A longer limit | With `"timeout": 86400` the waiter was still alive at 698 s and woke the idle session when triggered. Limits above 12 minutes were not run to the end. | Ship `86400`; an idle session is re-armed by any turn, so the worst case is one missed wake after a day of idleness. |
| `SessionStart` entry | Arms before the first prompt and wakes the session. | Register the waiter on `SessionStart` and `Stop`. |
| Several stops | Every `Stop` starts another waiter; the earlier ones keep running (three alive at once). | The per-conversation lock in step 7 is required, not optional. |
| Session exits normally | Claude Code sends the waiters `SIGTERM`. | Clean. |
| Session killed (`SIGKILL`) | The waiter is orphaned; polling its parent process id catches it within a second. | Step 7 keeps the parent check. |
| Headless `claude -p` | **Does not exit** while a waiter runs, and a wake starts another turn in it. | The waiter must not arm when headless: `CLAUDE_CODE_ENTRYPOINT` is `sdk-cli` there and `cli` when interactive. |

OpenCode 2.3.0, a free hosted model, a logging wrapper around `corvus mcp`:

| Question | Result | Consequence |
|---|---|---|
| Does the id reach `corvus mcp`? | Yes, on every tool call: `_meta["ai.opencode/sessionID"] = "ses_…"`; `clientInfo.name = "opencode"`. Not in the environment. | The link is made at `register_session` from request metadata. |
| Wake from outside | `opencode run --session <id> "<note>"` started a turn in the idle session in ~3 s, and the open window showed it live. | The `opencode` adapter is `["opencode", "run", "--session", "{conversation}", "{note}"]`. |
| One MCP process per session? | **No.** The shared service runs one `corvus mcp` for all sessions (two sessions, one process, two ids). | `corvus mcp`'s "remembered session" must be kept per conversation, not per process. |
| Inherited environment | The service passed its own start-up environment to `corvus mcp` — including an unrelated `CLAUDE_CODE_SESSION_ID`. | Trust that variable only when `clientInfo` says the client is Claude Code. |

Still open: 1c (the `/clear` mechanism is feasible — both events fire in one process with the same `CLAUDE_PID`
— but the write path is not chosen), 1e (Codex 0.169.0 is installed; needs the owner to sign in).

From documentation and source only, not run: the OpenCode 2.x service API and MCP session metadata (the 2.3.0
CLI on this machine does have `run --session`, `api`, `service`); Codex `codex queue`, its shared daemon, hooks
and MCP metadata (Codex is not installed).

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0023 | An agent is tied to its tool conversation: registering links them, a resumed conversation gets the same agent and inbox, and hooks find their agent by conversation — so one user-level setup serves every session. | [0023](../decisions/0023-agent-identity-follows-conversation.md) |
| D0024 | Idle sessions are woken through each tool's own channel (a background hook for Claude Code, the tools' shared services for OpenCode and Codex); Corvus does not own terminals. | [0024](../decisions/0024-wake-through-the-tools-own-channel.md) |
| D0021 | Tools are supported through declarative adapters that pass only a fixed note, never message text; the OpenCode and Codex wake paths stay inside that pattern. | [0021](../decisions/0021-agent-adapters.md) |
| D0015 | The hook and the waiter are subcommands of the one binary and open the database directly, with no server. | [0015](../decisions/0015-process-topology-single-binary.md) |
