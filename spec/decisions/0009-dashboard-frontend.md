# D0009 — Dashboard in Svelte 5 with tree fragments and a sink-free bundle

- **Status:** Superseded by [D0019](0019-dashboard-frontend-two-carriers.md)
- **Date:** 2026-11-13
- **Plans:** [0006](../plans/0006-dashboard-and-desktop.md)
- **Spec:** [system/05](../system/05-dashboard.md#frontend-ui) · P7, brief §5 "Dashboard"

## Problem

The dashboard is a moderately rich single page: seven panels, polling, filters, forms, two themes. It renders
text written by agents and copied from trackers, which must never be interpreted as markup or script (P7). The
brief asks for a page built "with text nodes only", a strict CSP, and a test that the shell contains "no
untrusted-markup sinks". Which frontend technology satisfies that and stays maintainable?

## Options

1. **Svelte 5 (runes) + TypeScript + Vite**, compiled to a static bundle.
2. **Vanilla TypeScript** with a small `h()` helper over `createElement` / `textContent`.
3. **A Rust/WASM framework** (Leptos, Yew, Dioxus).
4. **React or similar** virtual-DOM framework.

## Trade-offs

### Svelte 5
- **For:** the owner's standing preference for new frontend code; `{value}` interpolation produces text nodes;
  compiles to a small static bundle with no runtime template parsing of data; reactivity suits a polling
  dashboard; scoped CSS compiles to a stylesheet, compatible with `style-src 'self'`.
- **Against:** by default the compiler builds static template skeletons through `innerHTML`, so a naive scan
  for sinks fails and Trusted Types would block it. Svelte offers a `fragments: 'tree'` compiler option that
  builds templates with DOM calls instead; this must be confirmed to yield a bundle with no HTML sinks. Adds a
  Node toolchain to a Rust repo.

### Vanilla TypeScript
- **For:** literally "text nodes only"; trivially auditable; no framework.
- **Against:** hand-written state management and DOM diffing for seven live-updating panels — more code and
  more bugs; contrary to the owner's stated frontend preference.

### Rust/WASM
- **For:** one language across the repo.
- **Against:** WASM needs `'wasm-unsafe-eval'` in the CSP, loosening the strict policy; larger bundle; slower
  iteration; less mature tooling for this kind of UI.

### React-style
- **For:** familiar, text-safe by default.
- **Against:** bigger runtime, no advantage over Svelte here, and not the owner's choice.

## Decision

**Svelte 5 with runes, plain Vite (no SvelteKit), `fragments: 'tree'`.** Safety does not rest on the framework
alone: the source lint bans `{@html}` and DOM HTML sinks, a test scans the built bundle for those sinks, the CSP
sets `require-trusted-types-for 'script'`, and a browser test proves a hostile message body renders as text.

## Consequences

- **Spike first** ([plan 0006](../plans/0006-dashboard-and-desktop.md) step 1): build a one-component page with
  `fragments: 'tree'`, run the bundle scan, and load it under the full CSP in Chromium and WebKit. If the bundle
  cannot be made sink-free, supersede this record with vanilla TypeScript (option 2) before building panels.
- Node and a package manager become build-time prerequisites for `make ui`; Rust builds consume `ui/dist`.
- No inline styles: dynamic styling goes through classes and CSS custom properties.

## Discussion

- 2026-11-13 — Initial record. The `fragments: 'tree'` option is taken from memory of the Svelte 5 compiler
  options and has not been exercised here; the spike exists to verify it.
- 2026-11-13 — Re-examined for a self-contained Rust binary. The framework choice stands; delivery changes (one bundle, two carriers, prebuilt). Superseded by D0019.
