# D0019 — Svelte 5 dashboard: one bundle, two carriers, prebuilt into the binary

- **Status:** Superseded by [D0020](0020-desktop-only-dashboard.md)
- **Date:** 2026-11-13
- **Supersedes:** [D0009](0009-dashboard-frontend.md)
- **Plans:** [0006](../plans/0006-dashboard-and-desktop.md)
- **Spec:** [system/05](../system/05-dashboard.md#frontend-ui) · P7, brief §5 "Dashboard"

## Problem

[D0009](0009-dashboard-frontend.md) chose Svelte 5 for a page served by a server. Two things changed: the
dashboard now also runs inside a self-contained Tauri app over IPC ([D0018](0018-self-contained-desktop.md)),
and Corvus must be buildable and installable as a Rust binary ([D0013](0013-storage-sqlite.md)). The frontend
choice has to be re-made against those: does a JavaScript framework still fit a Rust-binary product, and how
does one UI serve two hosts? The P7 requirements are unchanged: untrusted text is rendered as text only.

## Options

1. **Svelte 5 + TypeScript**, built to static assets that are committed and embedded in the Rust binaries.
2. **A Rust/WASM framework** (Leptos, Dioxus, Yew) so the whole product is Rust.
3. **Vanilla TypeScript** with a tiny DOM helper.
4. **A native Rust GUI** (egui, iced, Slint) instead of a web UI.

## Trade-offs

### Svelte 5, prebuilt
- **For:** the owner's standing frontend preference; text interpolation yields text nodes; small static output;
  the same bundle runs in the app and in a browser. Tauri embeds frontend assets in the executable, so **end
  users and `cargo install` users never need Node** — provided the built assets are in the repository.
- **Against:** contributors changing the UI need Node; committed build output must be kept in step with its
  source (a CI check); request/response types exist in Rust and TypeScript unless generated; by default the
  Svelte compiler creates static template skeletons through `innerHTML`, which the sink scan and Trusted Types
  would reject — the `fragments: 'tree'` compiler option is expected to avoid this and must be verified.

### Rust/WASM
- **For:** one language; API types shared directly with `corvus-core`.
- **Against:** does not remove the build-tool problem — it swaps Node for a WASM toolchain and bundler, and
  `cargo install` still needs prebuilt assets; WASM requires `'wasm-unsafe-eval'` in the CSP, weakening the
  strict policy; larger payload; these frameworks also build static templates through HTML strings; slower UI
  iteration.

### Vanilla TypeScript
- **For:** literally "text nodes only"; no framework to audit.
- **Against:** hand-rolled state and DOM updates for seven live panels; still needs a TypeScript build.

### Native Rust GUI
- **For:** no web stack, no markup-injection class of bug at all.
- **Against:** the owner chose Tauri, which hosts web frontends; the brief requires a `/ui` web page, so a
  second UI would have to be written anyway.

## Decision

**Svelte 5 with runes, plain Vite (no SvelteKit), `fragments: 'tree'`.** The move to a self-contained Rust
binary does not change the answer, because the framework runs at build time, not on the user's machine. What
changes is how it is delivered:

- **One bundle, two carriers.** `ui/src/lib/api.ts` is the only data-access module. In the Tauri app it sends
  requests with `invoke('api', …)`; in a browser it uses `fetch` with the bearer token. Nothing above it knows.
- **Prebuilt and committed.** `ui/dist` is committed; `corvus-http` and `corvus-desktop` embed it at compile
  time. `cargo build` and `cargo install` need only Rust. CI rebuilds the UI and fails if `ui/dist` differs.
- **Generated types.** TypeScript request/response types are generated from the Rust API structs (`ts-rs`) and
  checked in; CI fails on drift.

Safety is layered and independent of the framework: source lint banning `{@html}` and DOM HTML sinks, a scan of
the built bundle, a strict CSP (with Trusted Types where the engine enforces it), and a browser test that
proves hostile text renders literally.

## Consequences

- **Spike first** ([plan 0006](../plans/0006-dashboard-and-desktop.md) step 1): confirm a `fragments: 'tree'`
  build is sink-free and runs under the full CSP in Chromium, WebKit and the Tauri webview with IPC. If it
  cannot be made sink-free, supersede this record with vanilla TypeScript before building panels.
- No inline styles; dynamic styling through classes and CSS custom properties.
- Committed build output adds review noise; it is marked generated in `.gitattributes`.

## Discussion

- 2026-11-13 — Created with the move to a self-contained app. Supersedes D0009; the framework is unchanged, the
  delivery model is new. The `fragments: 'tree'` option is from memory of the Svelte 5 compiler options and has
  not been exercised here; the spike exists to verify it.
- 2026-11-13 — The owner chose a desktop-only dashboard. Svelte and the text-safety layers carry forward; the HTTP carrier, the browser sign-in flow and committed `ui/dist` are removed. Superseded by D0020.
