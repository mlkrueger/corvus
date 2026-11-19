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

- [x] 1. **Spikes (gate — results recorded below before step 2).**
  - [x] a. Claude Code waiter edge cases: wake while the user is typing; wake while a turn is running; the
        longest a background hook may run and what happens at its timeout; whether a `SessionStart` entry arms
        before the first turn; a headless `claude -p` run must still exit promptly with the waiter registered.
  - [x] b. Waiter lifetime: it exits when its `claude` process is gone (killed, crashed, terminal closed).
  - [x] c. `/clear` carry-over: mechanism chosen (a note at `SessionEnd reason=clear`, one write at
        `SessionStart source=clear`; [system/06](../system/06-delivery-helpers.md#session-events)); proved on a
        real session in the by-hand checks once step 6 is built.
  - [x] d. OpenCode 2.3 live: the session id reaches `corvus mcp` (`_meta["ai.opencode/sessionID"]`);
        `opencode run --session <id> "<note>"` starts a turn in an idle open session and the open window shows it.
  - [x] e. *(pinned by the owner 2026-11-16 — Codex 0.169.0 is installed but not signed in; come back to it)*
        Codex: `codex queue --thread <id> --message "<note>"` on an idle
        open session; where the id reaches `corvus mcp`.
- [x] 2. **Spec first.** system/02 (table), /03 (service), /04 (`register_session`, MCP), /06 (hook identity,
      waiter, adapters bound from links), /08 (threats), /11 (defaults); accept or amend D0023 and D0024.
- [x] 3. Migration `0007_conversations`: `(tool, conversation_id) → agent_name`, working directory, timestamps.
      Link and carry-over written in one transaction with an audit event.
- [x] 4. `register_session` takes an optional conversation reference: known → attach to its agent; unknown →
      register as today and record the link. `corvus mcp` fills it from the environment or request metadata.
- [x] 5. `corvus hook` resolves its agent by conversation when `CORVUS_AGENT_NAME` is absent; no link → silent
      exit 0. Explicit name still wins.
- [x] 6. `/clear` carry-over, by the mechanism chosen in 1c.
- [x] 6b. Migration `0008`: `messages.wake`, `messages.reassigned_from`, `receipts.kind` / `reassigned_to`.
      `post_message` accepts `wake` on an operator broadcast only; `corvus tell --all [--wake]`.
- [x] 7. `corvus hook wait`: one waiter per (agent, conversation) under a file lock (a second exits 0); blocks
      on the inbox for mail not in the shown-set; exit 2 with the formatted messages on stderr and the shown-set
      updated; exit 0 when its session is gone; never fails in any other way.
- [x] 8. Setup: `corvus setup claude [--remove] [--yes]`; the same snippet in
      [docs/delivery-helpers.md](../../docs/delivery-helpers.md) and the app's "Connect an agent" view; README.
- [x] 9. Stranded mail: `corvus tell` warns when the recipient is not reachable; `reassign_mail` with
      `corvus mail reassign` and a dashboard action ([D0025](../decisions/0025-operator-reassigns-stranded-mail.md));
      the dashboard flags unreachable agents with unread mail, shows each conversation's directory, and has a
      "wake everyone" option on a broadcast.
- [x] 10. Built-in `opencode` adapter, bound from the conversation link; the background leader's wake step, so
      a running app is all that is needed.
- [x] 11. *(pinned with 1e)* `codex` adapter wakes with `codex queue` (enabled only if 1e passed; otherwise stays
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
- [x] By hand, real Claude Code, user-level hooks: two sessions in one folder are two agents; `corvus tell` to
      one wakes only that one; quit and `claude --resume` → same agent, unread mail delivered; `/clear` keeps
      the agent; a session that never connected is untouched.
- [x] By hand, OpenCode (and Codex if installed): tell → the idle session starts a turn and reads its inbox.
- [ ] `corvus tell` to an agent with no live session prints the offline warning; the dashboard flags it.

## Exit criteria

- [x] `make lint`, `make test` and `make test-e2e` pass.
- [ ] The by-hand Claude Code checks are recorded below with a date.
- [x] Each built-in wake path is either confirmed on a real session or listed as unverified and not enabled.

## Open questions

- **Resolved** (recorded in [defaults](../system/11-defaults.md) rows 60–66): the carry-over mechanism (63),
  the waiter time limit (61), and by the owner on 2026-11-16: direct mail and operator-marked broadcasts wake
  (62), the background leader wakes OpenCode agents (64), unreachable agents get a warning, a flag and
  reassignment (65, D0025), setup by command and by snippet (66).
- **Resolved in step 4:** Claude Code 2.1.325 sends no `initialize` request to a stdio server on protocol
  `2026-07-28`; it names itself (`claude-code`) in every request's `_meta["io.modelcontextprotocol/clientInfo"]`.
  OpenCode names itself at `initialize`. The server reads both.
- **Resolved in step 10:** killing `opencode run` mid-turn does not cancel the turn (the service runs it), so
  the adapter's timeout is the maximum, 3600 s.
- **Codex hooks**: Codex 0.169.0 has hooks in Claude Code's format (`session_id` = thread id). `corvus hook`
  does not serve them yet (it looks conversations up under `claude-code`), so a Codex agent sees mail only when
  woken or when it reads its inbox. Candidate for a later plan.
- **Headless Codex wake**: with no window open, `codex queue --resume` runs the turn in Codex's own background
  service, which reads `~/.codex/config.toml`. Confirmed with and without a window (by-hand results below).
- **`--fork-session`** is assumed to issue a new id (a new agent); untested.
- **Resolved by the owner 2026-11-19, built in [plan 0015](0015-operator-voice-recall-login.md):** operator
  mail is the owner speaking and Corvus says so ([D0028](../decisions/0028-operator-mail-is-the-owner.md));
  after `/clear` or a resume `corvus mcp` recalls the agent from the conversation link (defaults row 67); the
  desktop app launches at login (row 68). Codex was signed in and step 1e run on 2026-11-16 (results below).

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
| Kill `opencode run` mid-turn (step 10) | The turn carried on and finished 21 s after the kill. | A timeout is not a failed delivery; the adapter's limit is 3600 s. |
| The service's environment | The shared service keeps the environment of whoever started it; a bad `CORVUS_API_TOKEN` there made `corvus mcp` fail to start. | Documented as a troubleshooting note only. |

Codex 0.169.0 (step 1e, 2026-11-16), cheapest model, a logging wrapper around `corvus mcp`:

| Question | Result | Consequence |
|---|---|---|
| Identity | `initialize` clientInfo `codex-mcp-client`; every tool call has `_meta.threadId` (a UUID) and the same id in `_meta["x-codex-turn-metadata"].thread_id`. Nothing in the environment. One MCP process per Codex process; `codex resume <id>` keeps the id. | The link is made at `register_session` from request metadata. |
| Wake, window open | `codex queue --thread <id> --message "<note>"` returns in ~30 ms; the idle window ran the turn 6–9 s later, live. | The adapter uses `codex queue`. |
| Wake, no window | Without `--resume` the message just sits queued. With `--resume` Codex's service ran it headless within ~2 s. With a window open `--resume` only prints a warning. | The adapter passes `--resume`. |
| Old adapter (`codex exec resume`) on an open thread | Fails in 0.5 s: "already has an active writer". | Replaced. |
| Tool approval | Without pre-approval a non-interactive turn fails the tool call; `mcp_servers.corvus.default_tools_approval_mode = "approve"` fixes it. | In the docs. |
| Hooks | Present, Claude Code's format, `session_id` = thread id; `UserPromptSubmit` output reaches the model. | Not used yet (open question). |

### By-hand results, steps 8–11 (2026-11-16)

- **Setup (8):** `corvus setup claude --file <project settings>` on a file with an existing permission rule and
  an unrelated hook kept both and wrote the backup; a Claude Code 2.1.325 session using the generated entries
  was woken by `corvus tell`, replied and acked, with no permission stop (`"mcp__corvus"` allowed) and no
  waiter left after `/exit`. **Not run against the real `~/.claude/settings.json`.**
- **Stranded mail (9):** on a temp state directory: tell to an agent with an ended session warned and still
  delivered; `corvus mail reassign` moved the message once (second run: nothing); the new agent's inbox showed
  the copy with `reassigned_from`. **The dashboard parts are unit-tested only; not looked at on screen.**
  Reachability notes: an un-ended session whose process died counts as reachable until auto-end; a waiter
  running under an explicit `CORVUS_AGENT_NAME` on an unlinked conversation does not count.
- **OpenCode (10):** `corvus serve` as leader; an OpenCode 2.3.0 window registered as `oc-bob`; the leader
  started its watcher 3 s later; a plain broadcast did not wake it; `corvus tell oc-bob "What is 6*7? …"` got
  `42` in-thread 18 s later and an ack.
- **Codex (11):** same with a Codex 0.169.0 window registered as `cx-bob`: watcher 5 s after registering,
  `42` and an ack 16 s after the tell. The reply was posted as a broadcast in the thread rather than with
  `in_reply_to` (the model's choice).
- **Gates:** `make lint`, 575 Rust tests, 119 UI unit tests, the bundle scan and `make test-e2e` (46) pass.
- **Real settings (2026-11-16, evening):** after `corvus setup claude` on the owner's `~/.claude/settings.json`
  and the Corvus server in `~/.codex/config.toml`, on `~/.corvus`, Claude Code sessions driven in a
  pseudo-terminal:
  - Two sessions in one folder registered as two agents (`rc-alice`, `rc-bob`), two conversation links.
  - `corvus tell` woke only the addressed idle session, within 3-4 s.
  - A session told only "register and answer READY" woke but **declined to act** on the operator's mail and
    did not ack it (it treated the mail as a peer request outside its instructions). A session told that
    operator mail comes from the owner replied `42` and acked in 3 s. Whether Corvus's own guidance should say
    this is an open question below.
  - `claude --continue` after quitting: same agent (`rc-carol`).
  - `/clear`: the link was carried to the new id; `register_session` with no name returned the same agent
    (`rc-dave`) and the next tell woke it (4 s). Before re-registering, tool calls fail with "`agent` is
    required (register_session first)": the MCP server remembers its agent only in-process and nothing tells
    the cleared session to register again.
  - Codex with no window: `codex exec` registered `cx-real`; with `corvus serve` as leader the watcher started
    within 10 s and a tell got `44` back in 18 s, run by Codex's background service.
- **Not yet done:** the desktop app (rather than `corvus serve`) as the leader; the dashboard's new parts on
  screen; `--fork-session`. The app the owner launched was using a different state directory
  (`/private/tmp/corvus-manual`) from the CLI (`~/.corvus`), so it could not lead for these sessions.
| Inherited environment | The service passed its own start-up environment to `corvus mcp` — including an unrelated `CLAUDE_CODE_SESSION_ID`. | Trust that variable only when `clientInfo` says the client is Claude Code. |

### Steps 3–6 on a real session (2026-11-16)

Claude Code 2.1.325, interactive, release `corvus`, isolated state directory, `corvus hook` registered with **no
agent name** on `SessionStart`, `SessionEnd`, `UserPromptSubmit` and `Stop`, `corvus mcp` as the MCP server:
the model called `register_session` → agent `solar-raven` linked to the conversation with its directory;
`corvus tell solar-raven …` from outside → the next prompt was shown the message and the model answered with
its code; `/clear` → `conversation.carried`, the new id resolves to `solar-raven`, and the next prompt was shown
the unread message again (the hook's state file for the new id records it). Not yet run with the hooks in the
real user-level `~/.claude/settings.json`.

### Step 7 on a real session (2026-11-16)

Same setup, plus `corvus hook wait` (`asyncRewake`, `timeout` 86400) on `SessionStart` and `Stop`, Corvus MCP
tools pre-approved:

- **Idle wake, end to end:** the session registered and went idle; `corvus tell <agent> --type question "What
  is 17 + 25? …"` from outside → with nobody typing, the agent posted `42` as an `answer` in the same thread
  and acknowledged the question (one `ack` receipt).
- **Timeline run:** one waiter while idle; a direct message woke it (shown-set gained the id, a new waiter
  armed after the turn); an ordinary broadcast did **not** wake it; a `--wake` broadcast did, and showed the
  ordinary one with it. No waiter was left running after `/exit`.
- With the Corvus tools *not* pre-approved the woken turn stops at Claude Code's permission prompt — the setup
  docs must say to allow them.
- The first waiter of a brand-new state directory exits at `SessionStart` because no database exists yet; the
  `Stop` after the first turn arms it. Harmless, and only on a machine's very first session.

Earlier note, now superseded by step 6: 1c (the `/clear` mechanism is feasible — both events fire in one process with the same `CLAUDE_PID`
— but the write path is not chosen), 1e (Codex 0.169.0 is installed; needs the owner to sign in).

From documentation and source only, not run: the OpenCode 2.x service API and MCP session metadata (the 2.3.0
CLI on this machine does have `run --session`, `api`, `service`); Codex `codex queue`, its shared daemon, hooks
and MCP metadata (Codex is not installed).

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0023 | An agent is tied to its tool conversation: registering links them, a resumed conversation gets the same agent and inbox, and hooks find their agent by conversation — so one user-level setup serves every session. | [0023](../decisions/0023-agent-identity-follows-conversation.md) |
| D0024 | Idle sessions are woken through each tool's own channel (a background hook for Claude Code, the tools' shared services for OpenCode and Codex); Corvus does not own terminals. | [0024](../decisions/0024-wake-through-the-tools-own-channel.md) |
| D0025 | The operator can move an unreachable agent's unread mail to another agent: copies for the new agent, a "reassigned" mark on the originals. It is a transfer, not an acknowledgement, and not available to agents. | [0025](../decisions/0025-operator-reassigns-stranded-mail.md) |
| D0021 | Tools are supported through declarative adapters that pass only a fixed note, never message text; the OpenCode and Codex wake paths stay inside that pattern. | [0021](../decisions/0021-agent-adapters.md) |
| D0015 | The hook and the waiter are subcommands of the one binary and open the database directly, with no server. | [0015](../decisions/0015-process-topology-single-binary.md) |
