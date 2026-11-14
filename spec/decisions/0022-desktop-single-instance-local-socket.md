# D0022 — The desktop app's single-instance guard may use a local OS rendezvous; it never opens a network listener

- **Status:** Accepted
- **Date:** 2026-11-14
- **Plans:** [0006](../plans/0006-dashboard-and-desktop.md)
- **Spec:** [system/05](../system/05-dashboard.md#desktop-app-cratescorvus-desktop-tauri-2) · [system/08](../system/08-security.md) · P6

## Problem

[D0020](0020-desktop-only-dashboard.md) and system/05 say the desktop app "never opens a socket". The
single-instance guard (`tauri-plugin-single-instance`) needs a rendezvous so a second launch can find the first
and focus its window. On macOS that rendezvous is a Unix-domain socket at `/tmp/dev_corvus_desktop_si.sock`; on
Linux it is a well-known name on the D-Bus session bus. Neither is a network port, but the macOS one is a
listening socket, so the build contradicts the spec's wording. The question is whether to keep it.

## Options

1. **Keep the plugin; narrow the rule** to "no network listener" and allow a local OS rendezvous.
2. **Replace the plugin with a lock file** in the state directory: a second launch sees the lock and exits.
3. **Drop single-instance** entirely.

## Trade-offs

### Keep the plugin, narrow the rule
- **For:** a second launch (and `corvus ui`) focuses the existing window on every platform, as system/05 asks.
  The socket is reachable only from this machine; the payload (the second instance's arguments and working
  directory) is ignored — the only effect of a connection is "show the window". It carries no data and no API.
- **Against:** the spec's absolute wording becomes a narrower one. The macOS path is fixed in shared `/tmp`, so
  another local user who creates that path first can stop the app from starting (it would connect and exit).
  This needs a second account on the machine acting against you.

### Lock file
- **For:** no listening endpoint of any kind; the original wording stands.
- **Against:** a second launch can only exit, not focus the window — unless we add our own signal channel,
  which is the plugin again. `corvus ui` loses "focuses the existing window". Stale-lock handling after a crash.

### No single-instance
- **For:** least code.
- **Against:** two app processes on one database: both are valid Corvus processes, but two trays and two
  windows is confusing, and it contradicts system/05.

## Decision

**We chose to keep the plugin and narrow the rule (owner direction, 2026-11-14).** The property the rule exists
to protect is P6: nothing on the network, nothing that serves data without a token. A local rendezvous that can
only raise a window does not weaken that, and the focus-on-relaunch behaviour is worth keeping. The lock file
loses that behaviour for a wording gain only.

## Consequences

- The rule everywhere is now: **the desktop app never opens a network listener** (no TCP/UDP port, with or
  without a token). A local OS rendezvous used only to focus the window is allowed.
- The `no_listener` test keeps asserting no TCP/UDP socket; it does not assert the absence of the Unix socket.
- The single-instance callback must keep ignoring its arguments and working directory. If it ever acts on them,
  revisit this record (they come from any local process).
- Known gap: on a shared Mac, another user can squat the fixed `/tmp` path. Revisit if Corvus is ever meant
  for multi-user machines (for example a per-user path in `$TMPDIR`, which needs a plugin change).

## Discussion

- 2026-11-14 — Raised as an open owner decision in the plan 0010 final report; owner chose to keep the socket.
