# Delivery helpers: the hook, waking idle agents, and adapters

Agents pull their mail. These helpers make pulling reliable. None of them acknowledges a message: only the agent
does that. None needs a server or a token. Design: [spec/system/06](../spec/system/06-delivery-helpers.md).

## Claude Code: show new mail with the hook

`corvus hook` reads the Claude Code hook event on stdin and, when there is new mail for the agent, adds it to the
conversation (on a prompt or after a tool call) or keeps Claude from stopping once (on stop). It never fails: on any
problem it prints nothing and exits 0.

Add to `.claude/settings.json` (project) or `~/.claude/settings.json` (user). Put the agent name in the command —
Claude Code does not document passing settings `env` to hooks:

```json
{
  "hooks": {
    "UserPromptSubmit": [
      { "hooks": [{ "type": "command", "command": "CORVUS_AGENT_NAME=my-agent corvus hook" }] }
    ],
    "PostToolUse": [
      { "matcher": "*", "hooks": [{ "type": "command", "command": "CORVUS_AGENT_NAME=my-agent corvus hook" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "CORVUS_AGENT_NAME=my-agent corvus hook" }] }
    ]
  }
}
```

Use the same name the agent registers with (`register_session` / `CORVUS_AGENT_NAME` for `corvus mcp`).
After-tool checks are throttled to one every `CORVUS_HOOK_THROTTLE_SECONDS` (default 5). Each message is shown
once per Claude session, at most 10 at a time.

## Waking an idle agent: `corvus watch`

`corvus watch <agent>` waits on the inbox (no model tokens spent) and, when new mail arrives:

| Mode | How | What happens |
|---|---|---|
| adapter (default for a bound agent) | `corvus agent bind …` first | runs the tool's adapter with a fixed note — never the message text |
| exec | `corvus watch <agent> --exec <cmd> [args…]` (last on the line) | runs your command with the messages as JSON on stdin and in `$CORVUS_WAKE_MESSAGES_FILE` |
| print (default when unbound) | `corvus watch <agent> --print` | prints the unread messages as JSON and exits |

Exit 0 from the command means "delivered" (not "handled"); failures are retried with backoff 1 s, 2 s, 4 s … up to
300 s; a command running longer than `--timeout` (default 300 s) is killed with its children. One watcher per
agent. Restarting resumes where it left off; a crash at the wrong moment can deliver a batch twice (never zero
times).

### Binding an agent to its tool

```sh
corvus adapter list                     # built-ins and your own adapters
corvus adapter check codex              # is the tool installed?
corvus agent bind my-agent --adapter codex --conversation 0199a2b4-1c2d-7e8f-9a0b-1c2d3e4f5a6b
corvus agent list
corvus up                               # starts a watcher for every bound agent (and corvus serve if a token is set)
corvus status
corvus down
```

For Codex, the conversation id is the session id of the Codex conversation you want resumed. Waking runs
`codex exec resume <id> "<note>"`, which adds a turn to that stored conversation.

## Supporting a new tool: write an adapter

A tool can be woken if it can be given a prompt for an existing session from the command line. Supporting it is
one TOML file — no Rust changes, no rebuild.

1. Find the tool's resume command, e.g. `mytool resume --session <id> --prompt "<text>"`.
2. Write `~/.corvus/adapters/mytool.toml` (mode `0600` or `0644`, owned by you, not a symlink):

   ```toml
   name  = "mytool"
   title = "My Tool"

   [binding]
   conversation = { required = true, pattern = "chars:abcdefghijklmnopqrstuvwxyz0123456789-:8-64" }

   [wake]
   command = ["mytool", "resume", "--session", "{conversation}", "--prompt", "{note}"]
   timeout_seconds = 120

   [check]
   command = ["mytool", "--version"]
   ```

   - Placeholders are exactly `{agent}`, `{conversation}`, `{note}`, `{count}`, `{state_dir}`, each a whole
     argument (`--prompt={note}` is rejected).
   - There is no placeholder for message text. `{note}` is always: *"You have unread Corvus messages. Check your
     Corvus inbox (read_inbox), handle them, and ack each one."*
   - `pattern` is `"uuid"` or `"chars:<allowed characters>:<min>-<max>"`.
   - The command runs without a shell; exit 0 means the tool accepted the nudge.
3. Check and bind it:

   ```sh
   corvus adapter show mytool
   corvus adapter check mytool
   corvus agent bind my-agent --adapter mytool --conversation <id>
   corvus watch my-agent          # or: corvus up
   ```

A user adapter with the same name as a built-in replaces it. To ship one as a built-in, add the file under
`crates/corvus-core/adapters/` with a test that runs it against a stub of the tool.
