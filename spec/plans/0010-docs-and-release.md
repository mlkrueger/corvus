# Plan 0010 — Docs, `DECISIONS.md`, full verification, honest report

- **Status:** Ready
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

- [ ] 1. `README.md`: what it is, quick start (download or `cargo install`, then `corvus mcp` in the agent's
      config — or open the desktop app), connecting an agent (MCP + hook), running tests. Explains in plain words when `corvus serve` is needed (headless background work, HTTP API) and why it requires a token even though it is local. Includes the guidance text; a test fails if it drifts from `GUIDANCE`.
- [ ] 2. `docs/usage.md`: routes, CLI commands and MCP tools, generated or checked against the code where possible.
- [ ] 3. `docs/architecture.md`: the design in brief, decisions, **known gaps** (from
      [system/01](../system/01-architecture.md#known-gaps-kept-current-feeds-the-final-architecture-note)).
- [ ] 4. Root `DECISIONS.md`: index of [spec/decisions](../decisions/README.md) (one line each, linked) plus the
      [defaults table](../system/11-defaults.md). A test checks every decision record is listed.
- [ ] 5. Reconcile: walk each system-spec file against the code; fix the spec where the build diverged; make
      sure every plan's status, checkboxes and "Unverified / deferred" section are current.
- [ ] 6. Full run from a clean checkout: `make lint`, `make test`, `make test-e2e`; capture results.
- [ ] 7. Fresh-machine walkthrough of the README by someone (or an agent) with no context.
- [ ] 8. Final report: what passes (with counts), what was not built, what was verified only by hand, what
      could not be verified.

## Verification

- [ ] Docs tests pass (guidance copy, decisions index completeness, no secret-looking strings in the repo).
- [ ] A link checker finds no broken relative links under `spec/`, `docs/` and the root docs.
- [ ] Every row of the [required test matrix](../system/09-testing.md#required-test-matrix) maps to named, passing tests.
- [ ] The README quick start works verbatim on a clean machine with nothing but the released binary.

## Exit criteria

- [ ] The final report exists and makes no claim that was not exercised.
- [ ] All plans are `Done` or explicitly `Abandoned` with a reason.

## Open questions

- None.

## Unverified / deferred

—

## Decisions

| ID | Summary | Record |
|---|---|---|
| all | This plan publishes the whole decision set (current and superseded) as the root `DECISIONS.md`; it relies on no decision of its own. | [index](../decisions/README.md) |
