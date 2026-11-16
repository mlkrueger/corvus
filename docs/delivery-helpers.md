# Delivery helpers: the hook, waking idle agents, and adapters

Agents pull their mail. These helpers make pulling reliable. None of them acknowledges a message: only the agent
does that. None needs a server or a token. Design: [spec/system/06](../spec/system/06-delivery-helpers.md).

## Claude Code: show new mail and wake idle sessions

Two commands run as Claude Code hooks:

- `corvus hook` reads the hook event on stdin and, when there is new mail for the session's agent, adds it to the
  conversation (on a prompt or after a tool call) or keeps Claude from stopping once (on stop).
- `corvus hook wait` runs in the background while the session sits at its prompt. It costs no model tokens. When
  mail addressed to the agent arrives (or the operator sends `corvus tell --all --wake …`), it wakes the session
  with that mail. An ordinary broadcast does not wake anyone; it is shown at the next turn.

Neither ever fails: on any problem they print nothing and exit 0. Neither marks anything read.

Both work out which agent a session is from the conversation: the agent that session registered as through the
Corvus MCP server. A session that never registered is left alone, so one user-level setup is safe for every
session on the machine. Resuming a conversation keeps its agent; so does `/clear`.

### Set it up

```sh
corvus setup claude            # shows the change to ~/.claude/settings.json and asks before writing
corvus setup claude --remove   # takes the Corvus entries out again
```

- `--yes` skips the question. `--file <path>` edits another settings file (for example a project's
  `.claude/settings.json`); `CLAUDE_CONFIG_DIR` is honoured.
- It touches only its own entries — commands that are exactly `<path to corvus> hook` or `… hook wait` — and keeps
  everything else, in the same order. The file is re-indented with two spaces if it was not already.
- The previous file is copied to `settings.json.corvus-backup` beside it. A file that is not valid JSON is refused.
- The entries use the full path of the `corvus` binary that ran the command. If you move the binary, run the
  command again: it replaces the old entries.
- Sessions that are already running pick the hooks up at their next event.

Or paste it yourself — this is the same change, with `corvus` found on `PATH` (use the full path if Claude Code
does not see your `PATH`):

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "corvus hook"
          },
          {
            "type": "command",
            "command": "corvus hook wait",
            "asyncRewake": true,
            "timeout": 86400
          }
        ]
      }
    ],
    "SessionEnd": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "corvus hook"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "corvus hook"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "corvus hook"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "corvus hook"
          },
          {
            "type": "command",
            "command": "corvus hook wait",
            "asyncRewake": true,
            "timeout": 86400
          }
        ]
      }
    ]
  }
}
```

### Let a woken session answer on its own

A woken session starts a turn with nobody at the keyboard. If the Corvus tools need approval, that turn stops at
the permission prompt. Register the MCP server for every project and allow its tools:

```sh
claude mcp add --scope user corvus -- corvus mcp
```

```json
{ "permissions": { "allow": ["mcp__corvus"] } }
```

(`mcp__corvus` allows every tool of the MCP server named `corvus`; add it to the `allow` list you already have.)

### Check it

```sh
corvus tell my-agent "hello"     # my-agent: the name the session registered with
```

An idle session starts a turn and shows the message within a second or so. A busy one shows it after its current
tool call. If nothing happens:

| Symptom | Likely cause |
|---|---|
| nothing, ever | the session never called `register_session` (no agent is linked to it), or the hooks are not in the settings file that session reads |
| shown at the next prompt, but an idle session is not woken | the `corvus hook wait` entries are missing `"asyncRewake": true` or the `"timeout"` (Claude Code stops a background hook after 10 minutes by default) |
| woken, then waits | the Corvus tools are not allowed (above) |
| the first session on a new machine is not woken until its first turn | the database did not exist yet when the session started; it re-arms after every turn |

Headless runs (`claude -p`) are never woken — a waiter would keep them from exiting.

After-tool checks are throttled to one every `CORVUS_HOOK_THROTTLE_SECONDS` (default 5). Each message is shown
once per conversation, at most 10 at a time. Hook state is under `~/.corvus/hook/`.

### A fixed name instead

To pin a project's sessions to one agent name whatever they register as, put the name in the hook command in
that project's `.claude/settings.json` (Claude Code does not document passing settings `env` to hooks):

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

`corvus setup claude` leaves such entries alone. Do not combine them with the user-level setup for the same
session unless the names agree. Test one by hand — this prints the JSON Claude Code would receive, or nothing:

```sh
corvus tell my-agent "hello"
echo '{"hook_event_name":"UserPromptSubmit","session_id":"test1","prompt":"hi"}' | CORVUS_AGENT_NAME=my-agent corvus hook
```

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

You rarely need to bind by hand. An OpenCode or Codex session that has called `register_session` is linked to
its agent automatically, and **while the desktop app (or `corvus serve`) is running it is woken for you**: mail
addressed to the agent, or a broadcast sent with `corvus tell --all --wake`, starts a turn in that session
within a few seconds. `corvus agent bind` + `corvus up` are for headless machines and for overriding the link.

| Tool | How it is woken | What you need |
|---|---|---|
| OpenCode 2.x | `opencode run --session <id> "<note>"` — an open window shows the turn live | the Corvus MCP server in `opencode.json` |
| Codex ≥ 0.157 | `codex queue --thread <id> --resume --message "<note>"` — an open window runs it a few seconds later; with no window, Codex's background service runs it | see below |

Codex needs the Corvus tools pre-approved, or a woken turn stops at the approval step. In
`~/.codex/config.toml`:

```toml
[mcp_servers.corvus]
command = "corvus"
args = ["mcp"]
default_tools_approval_mode = "approve"
```

For an explicit Codex binding, the conversation id is the Codex thread id.

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
