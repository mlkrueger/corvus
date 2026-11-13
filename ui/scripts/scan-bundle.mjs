// Fails if the built bundle contains an HTML-injection sink, inline script/style, a style attribute, or an
// external URL (system/05 "Untrusted text"). Usage: node scripts/scan-bundle.mjs <dist dir>
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

const dir = process.argv[2] ?? 'dist';
const SINKS = [
  /\binnerHTML\b/, /\bouterHTML\b/, /\binsertAdjacentHTML\b/, /\bdocument\.write\b/,
  /\beval\s*\(/, /\bnew\s+Function\b/, /\bsetHTMLUnsafe\b/, /\bcreateContextualFragment\b/,
];
const HTML_ONLY = [/<script(?![^>]*\bsrc=)[^>]*>/i, /<style[\s>]/i, /\sstyle\s*=/i, /\son[a-z]+\s*=/i];
const EXTERNAL = /\b(?:https?:)?\/\/(?!localhost\b|127\.0\.0\.1\b)[a-z0-9.-]+\.[a-z]{2,}/i;
// Strings in library code that are names, not network origins (e.g. XML namespaces).
const ALLOWED_URLS = [
  /http:\/\/www\.w3\.org\//,
  // Svelte's runtime prints links to its error docs in console warnings; they are text, never loaded
  // (and the CSP forbids loading anything external).
  /https:\/\/svelte\.dev\/e\//,
];

// Svelte's runtime (set_attribute -> get_setters) mentions the sink names only as the right-hand side of a
// `key !== 'innerHTML'` exclusion, so spread/dynamic attributes can never reach them. That is a deny-list
// comparison, not a sink; it is stripped before scanning. Any other use (assignment, property access,
// bracket access, call) still fails.
const DENY_LIST_COMPARISON = /!==?\s*(['"`])(?:innerHTML|outerHTML)\1/g;

const hits = [];
function walk(d) {
  for (const name of readdirSync(d)) {
    const p = join(d, name);
    if (statSync(p).isDirectory()) { walk(p); continue; }
    if (!/\.(js|mjs|css|html)$/.test(name)) continue;
    const text = readFileSync(p, 'utf8');
    const sinkText = text.replace(DENY_LIST_COMPARISON, '');
    for (const re of SINKS) if (re.test(sinkText)) hits.push(`${p}: sink ${re}`);
    if (name.endsWith('.html')) for (const re of HTML_ONLY) if (re.test(text)) hits.push(`${p}: ${re}`);
    for (const m of text.matchAll(new RegExp(EXTERNAL, 'gi'))) {
      if (!ALLOWED_URLS.some((a) => a.test(m[0]) || a.test(text.slice(m.index, m.index + 40)))) hits.push(`${p}: external URL ${m[0]}`);
    }
  }
}
walk(dir);
if (hits.length) { console.error(hits.join('\n')); process.exit(1); }
console.log(`bundle scan: ok (${dir})`);
