# D0021 — Agent tools are supported through declarative adapters; Codex is the first built-in

- **Status:** Accepted
- **Date:** 2026-11-13
- **Plans:** [0007](../plans/0007-delivery-helpers.md)
- **Spec:** [system/06](../system/06-delivery-helpers.md#agent-adapters) · brief §4.7

## Problem

The brief describes one way to wake an idle agent — a Codex-specific callback for the wake supervisor — plus a
generic "run a configured command". The owner expects to add other agent tools later (for example OpenCode or
Pi) and wants a pattern for it: a way to **register** an agent that runs in some tool and have Corvus **wake**
it, without each new tool being a special case in the code. What is that pattern, and how much is needed to
add a tool?

## Options

1. **Status quo: `--exec` only.** Every tool is a user-written script passed to `corvus watch --exec`.
2. **Compiled-in adapters.** A Rust trait with one implementation per tool, shipped in the binary.
3. **Declarative adapters.** A small data file per tool describing how to wake it (a command template and what
   identifier it needs); built-ins are embedded in the binary, users add their own files.
4. **Executable plug-ins.** `corvus-adapter-<name>` programs found on `PATH`, speaking a small protocol.

## Trade-offs

### `--exec` only
- **For:** nothing to build; maximally flexible.
- **Against:** no notion of "this agent runs in tool X"; every user re-invents the script and its safety rules
  (most importantly: never put peer message text in a prompt); nothing for the desktop app to list or check.

### Compiled-in adapters
- **For:** typed, tested, can do anything.
- **Against:** adding or fixing a tool means a code change and a release; agent CLIs change their flags often,
  so users would be stuck waiting; the owner cannot add a tool on their own machine.

### Declarative adapters
- **For:** adding a tool is writing a short file, no rebuild; when a tool's CLI changes, the fix is a one-line
  edit; one code path runs every tool, so the safety rules are enforced once — the template has no placeholder
  for message text, so no adapter can leak it into a prompt; the app and `corvus status` can list adapters and
  check whether each tool is installed.
- **Against:** only covers tools that can be woken by running a command; a template language, however small,
  must be specified and validated; adapter files are executable configuration and must be trusted like one.

### Executable plug-ins
- **For:** unlimited flexibility with a stable contract.
- **Against:** every adapter is a program to write, distribute and trust; nothing stops one from forwarding
  message text; heavier than the problem.

## Decision

**Option 3, with option 1 kept as the escape hatch.** An adapter is a TOML file: a name, the identifier a
binding needs (with a validation pattern), a wake command as an argument list with a fixed set of placeholders,
a timeout, and an optional "is it installed" check. Built-in adapters (Codex first) are embedded in the binary;
user adapters live in `<state dir>/adapters/` and override built-ins of the same name. `corvus agent bind`
ties an agent name to an adapter and an identifier; `corvus watch` wakes a bound agent through its adapter.
`corvus watch --exec` remains for anything a template cannot express.

Anything that speaks MCP can already *connect* with `corvus mcp`; adapters are about *waking*.

## Consequences

- The brief's "Codex adapter" becomes the built-in `codex` adapter; the brief's rules for it (fixed note, no
  peer text, validated agent name and conversation id, delivery is not acknowledgement) become rules of the
  adapter runner and apply to every tool.
- Supporting a new tool needs: its adapter file, and — to ship it as a built-in — a test with a stub of its CLI.
- A tool that cannot be woken by a command (no headless "resume" of any kind) cannot have a wake adapter; such
  an agent still works, relying on its own inbox checks.
- Hook-style delivery (injecting mail into a running session, as for Claude Code) is a different capability
  and stays a dedicated subcommand; if a second tool gains comparable hooks, generalising it is a new decision.
- Adapter files run commands as the user: they are loaded only if owned by the user and not writable by others.

## Discussion

- 2026-11-13 — Created after the owner asked for an adapter pattern so tools such as OpenCode or Pi can be
  added later. No adapter for those tools is specified here: how each is woken must be checked against that
  tool when it is added.
