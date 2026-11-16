# Plan 0010 — Docs, `DECISIONS.md`, full verification, honest report

- **Status:** In progress
- **Depends on:** all previous plans
- **Spec:** [system/09](../system/09-testing.md#honesty-rule) · brief §7, §8

## Goal

A stranger can start Corvus from the README in a few commands, and the project ends with a truthful account of
what was built, tested and left unverified.

## Scope

**In:** `README.md`, `docs/usage.md` (API / CLI / MCP reference), `docs/architecture.md`, root `DECISIONS.md`,
final full-suite run and report, spec reconciliation.

**Out:** packaging and release artifacts ([0011](0011-distribution.md)); CI configuration (future plan).

## Work breakdown

- [x] 1. `README.md`: what it is, quick start (download or `cargo install`, then `corvus mcp` in the agent's
      config — or open the desktop app), connecting an agent (MCP + hook), running tests. Explains in plain words when `corvus serve` is needed (headless background work, HTTP API) and why it requires a token even though it is local. Includes the guidance text; a test fails if it drifts from `GUIDANCE`.
- [x] 2. `docs/usage.md`: routes, CLI commands and MCP tools, generated or checked against the code where possible.
- [x] 3. `docs/architecture.md`: the design in brief, decisions, **known gaps** (from
      [system/01](../system/01-architecture.md#known-gaps-kept-current-feeds-the-final-architecture-note)).
- [x] 4. Root `DECISIONS.md`: index of [spec/decisions](../decisions/README.md) (one line each, linked) plus the
      [defaults table](../system/11-defaults.md). A test checks every decision record is listed.
- [x] 5. Reconcile: walk each system-spec file against the code; fix the spec where the build diverged; make
      sure every plan's status, checkboxes and "Unverified / deferred" section are current.
- [x] 6. Full run from a clean checkout: `make lint`, `make test`, `make test-e2e`; capture results.
- [x] 7. Fresh-machine walkthrough of the README by someone (or an agent) with no context.
- [x] 8. Final report: what passes (with counts), what was not built, what was verified only by hand, what
      could not be verified.

## Verification

- [x] Docs tests pass (guidance copy, decisions index completeness, no secret-looking strings in the repo).
- [x] A link checker finds no broken relative links under `spec/`, `docs/` and the root docs.
- [x] Every row of the [required test matrix](../system/09-testing.md#required-test-matrix) maps to named, passing tests.
- [x] The README quick start works verbatim on a clean machine with nothing but the released binary.
      (2026-11-16, from the `make dist` archives on clean Linux images and a clean macOS home; see plan 0011.)

## Exit criteria

- [x] The final report exists and makes no claim that was not exercised ([docs/report.md](../../docs/report.md)).
- [ ] All plans are `Done` or explicitly `Abandoned` with a reason. (0006 and 0007 are Done; 0011 remains *In progress* pending the owner's clean-account check and publishing.)

## Open questions

- None.

## Unverified / deferred

- **README quick start on a clean machine with only the released binary**: walked through by a no-context agent
  using the release binary in an isolated home directory on this Mac (not a separate clean machine). It found
  that step 3 failed (`tell` to an unknown agent); fixed and re-run. The Linux clean-container smoke test covers
  the binary itself.
- Step 5 reconciliation: `system/04` updated where the build diverged (error responses for 404/405/408/413,
  `WWW-Authenticate`, which routes take idempotency keys, `digest list`, `serve`/`mcp`/`hook` output);
  other spec files were updated plan by plan as the build went.
- The plan cannot reach "all plans Done" until the owner's manual checks for plans 0006, 0007 and 0011 are done.

## Decisions

| ID | Summary | Record |
|---|---|---|
| all | This plan publishes the whole decision set (current and superseded) as the root `DECISIONS.md`; it relies on no decision of its own. | [index](../decisions/README.md) |
