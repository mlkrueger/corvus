# Plan 0015 — Operator mail speaks for the owner; recall after `/clear`; launch at login

- **Status:** Done
- **Depends on:** [0012](0012-session-identity-and-wake.md), [0014](0014-app-updates.md)
- **Spec:** [system/04](../system/04-interfaces.md#mcp-server-corvus-mcp-stdio) ·
  [system/05](../system/05-dashboard.md#desktop-app-cratescorvus-desktop-tauri-2) ·
  [system/06](../system/06-delivery-helpers.md#inbox-hook-corvus-hook) · [system/08](../system/08-security.md)

## Goal

Three follow-ups to plan 0012, answered by the owner on 2026-11-19: a woken session acts on the operator's mail
because Corvus tells it the operator is its user; a session after `/clear` (or a resume) can use the Corvus
tools without registering again; the desktop app starts at login so the background leader is running after a
reboot.

## Scope

**In:**
- The guidance and the hook's message header say that mail from the operator is the user speaking; mail from
  any other agent stays a peer request. `corvus mcp` refuses to act as the operator ([D0028](../decisions/0028-operator-mail-is-the-owner.md)).
- `corvus mcp` recalls the agent from the conversation link when a call's conversation has no remembered
  session, and registers a session for it before running the call.
- The desktop app starts at login (on by default, once), opens into the menu bar only when started that way,
  and has a switch in the Connect view.

**Out:** Codex hooks (`corvus hook` for Codex) — still a candidate for a later plan; `--fork-session`.

## Work breakdown

- [x] 1. Spec: system/04 (recall, operator guard), /05 (launch at login), /06 (hook header), /08 (threat row),
      /11 (defaults rows 67–68); D0028.
- [x] 2. Operator voice: `GUIDANCE` rule 1 and the hook header name the operator as the user; the header
      labels operator mail "from the operator (your user)".
- [x] 3. Operator guard: `corvus mcp` rejects any call whose agent field (filled or explicit) is the operator
      name, including `register_session`.
- [x] 4. Recall: a call with a conversation but no remembered session looks up the link; when there is one it
      registers a session for that agent (the normal `connect`, with the conversation) and remembers it.
- [x] 5. Launch at login: `tauri-plugin-autostart` (macOS launch agent), Rust-side only; the app turns it on
      at its first launch and records that it did; `--autostart` starts it in the menu bar; commands
      `get_launch_at_login` / `set_launch_at_login`; a checkbox in the Connect view.

## Verification

- [x] Unit: the hook header and guidance name the operator; `corvus mcp` rejects `from = operator`,
      `register_session agent_name = operator` and a remembered agent that is the operator.
- [x] Unit (real SQLite file): a call on a linked conversation with no memory registers a session for the linked
      agent, links nothing new, and fills the call; an unlinked conversation still fails with "register_session
      first"; recall after a carried link (new id) finds the same agent.
- [x] By hand, real Claude Code: a session with narrow instructions woken by operator mail acts on it and acks.
- [x] By hand, real Claude Code: after `/clear`, a Corvus tool call without `register_session` works as the same
      agent. Record whether Claude Code restarted `corvus mcp`.
- [x] By hand, desktop app: first launch registers the login item; turning the switch off removes it and a
      relaunch does not turn it back on; a login-style launch (`--autostart`) shows no window.

## Exit criteria

- [x] `make lint` and `make test` pass (and `make test-e2e`).
- [x] The by-hand checks above are recorded below with a date.

## Open questions

- None yet.

## Unverified / deferred

### By-hand results (2026-11-19)

Claude Code 2.1.328, interactive, driven in a pseudo-terminal; release `corvus` from this branch on a throwaway
state directory; user settings excluded (`--setting-sources project,local`), hooks from `corvus setup claude
--file`, `mcp__corvus` allowed, MCP from `--mcp-config --strict-mcp-config`; default model.

- **Operator mail, narrow instructions** (the plan 0012 failure): told only "register, reply READY", the idle
  session was woken by `corvus tell <agent> --type question "What is 6*7? …"`, answered `42` in the thread in
  4 s and acked it.
- **`/clear`:** `corvus mcp` was **not** restarted (same pid before and after); the hook carried the link to
  the new id; `get_bootstrap_context` with no `register_session` returned the same agent, served by the
  session the process already remembered (no new session). A tell afterwards woke it and got `10` in 5 s. The
  plan 0012 failure after `/clear` (2.1.325, real settings) did not reproduce.
- **Resume (`claude --continue`), new MCP process:** `get_bootstrap_context` without `register_session`
  returned the linked agent with no error; `corvus mcp` recalled it and registered one new session
  (`session.registered`), which is the path step 4 adds.
- On `/exit` the first session's `corvus mcp` did not end its session (it stayed open until auto-end), so
  Claude Code stops the server without closing its stdin. Already covered by auto-end; not changed here.

### Desktop app by hand (2026-11-19)

The release bundle from this branch, on a throwaway state directory, after the owner quit the running instance:

- **First launch** wrote `~/Library/LaunchAgents/dev.corvus.desktop.plist` (`RunAtLoad`, the bundle's
  executable plus `--autostart`) and the marker `desktop/autostart-set`; the window was shown.
- **Unchecking "Start Corvus at login"** in the Connect view (clicked through macOS accessibility) removed the
  plist. Quit and relaunch: the plist stayed absent.
- **A launch with `--autostart`** put no window on screen (0 windows owned by the process); the app ran in the
  menu bar.
- Then the owner's app was relaunched on `~/.corvus`: it became the background leader (`corvus status`) and its
  first launch there turned the login item on (marker written, plist pointing at
  `target/release/bundle/macos/Corvus.app`).
- Not looked at: the checkbox's appearance (the session has no screen-recording permission); an actual
  log-out/log-in.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0028 | Mail from the operator is the owner speaking, and Corvus says so to agents; `corvus mcp` refuses to send or act as the operator, so an agent cannot borrow that voice through its tools. | [0028](../decisions/0028-operator-mail-is-the-owner.md) |
| D0023 | An agent is tied to its tool conversation; the recall in step 4 is that link used by the MCP server as well as the hook. | [0023](../decisions/0023-agent-identity-follows-conversation.md) |
