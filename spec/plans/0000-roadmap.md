# Plan 0000 — Roadmap

- **Status:** Ready
- **Spec:** [system/](../system/README.md) · brief §8

## Goal

Fix the build order and the milestone at which each part of Corvus is demonstrably working. The order follows
brief §8, with the work-source engine and Linear placed before Jira by owner direction, and a distribution plan
added because Corvus must ship as a self-contained binary.

## Build order

```
0001 foundation ─► 0002 Huginn ─► 0003 Muninn + Hermod ─► 0004 transports ─┬─► 0005 housekeeping + leader ─┬─► 0006 dashboard + desktop app
                                                                            │                               └─► 0008 source engine + Linear ─► 0009 Jira
                                                                            └─► 0007 delivery helpers
                                                    0006 + 0007 ─► 0011 distribution            everything ─► 0010 docs + release
```

0005 and 0007 are independent after 0004. 0008's dashboard panel needs 0006.

## Milestones

| # | After plan | You can… |
|---|---|---|
| M1 | 0001 | Run `corvus health` on a clean machine: it creates and migrates its own database. No setup. |
| M2 | 0002 | Run the concurrency suite — racing claimers across threads *and processes*, leases, cancellation |
| M3 | 0003 | Post, read, ack and wait across two processes; export a snapshot to a git repo |
| M4 | 0004 | Connect a real agent with just `corvus mcp`; drive everything from the CLI; `curl` the API when a token is set |
| M5 | 0005 | Leave `corvus serve` running; leases expire and silent sessions end on their own |
| M6 | 0006 | Double-click the desktop app and get a working dashboard with nothing else running |
| M7 | 0007 | Claude Code sees peer mail via the hook; a watcher wakes an idle agent; `corvus up/down/status` |
| M8 | 0008 | Linear issues assigned to you appear on the board and flow into the queue |
| M9 | 0009 | Same for Jira |
| M10 | 0011 | Download one binary (or the app) onto a clean machine and it works |
| M11 | 0010 | A stranger follows the README to a working system; the final report states exactly what was verified |

## Rules for every plan

- Keep it working at every step (brief §8): the workspace builds and the suite passes after each numbered step.
- Tests come with the code they cover, in the same step; concurrency tests run against real SQLite files.
- Verify by running: tests, then the real binary / endpoint / UI ([AGENTS.md §2](../../AGENTS.md#2-the-workflow)).
- Never add a runtime dependency on anything outside the binary ([AGENTS.md §5](../../AGENTS.md#5-engineering-rules-enforced)).
- When reality diverges from the system spec, fix the spec in the same change.

## Decisions

| ID | Summary | Record |
|---|---|---|
| D0013 | SQLite compiled into the binary is the single store, so Corvus needs no container or database server. | [0013](../decisions/0013-storage-sqlite.md) |
| D0015 | One `corvus` binary; every local process opens the database file directly, so agents need no running service and the HTTP API is opt-in. | [0015](../decisions/0015-process-topology-single-binary.md) |
| D0020 | The dashboard is the self-contained Tauri 2 app only: it embeds the core and reaches it over IPC through the same route table as the HTTP API. No browser UI. | [0020](../decisions/0020-desktop-only-dashboard.md) |
| D0011 | Linear and Jira are adapters on one work-source engine; Linear is built first. | [0011](../decisions/0011-work-source-abstraction.md) |
