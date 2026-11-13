// Belt-and-braces text scan of src/ (independent of ESLint): HTML-injection sinks, style attributes and the ack
// route must not appear (system/05 "Untrusted text", "Viewing never acks"). Generated types are skipped.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const root = process.argv[2] ?? 'src';
const SKIP_DIRS = new Set([join(root, 'lib', 'types')]);

const SINKS = [
  [/\{@html\b/, '{@html}'],
  [/\binnerHTML\b/, 'innerHTML'],
  [/\bouterHTML\b/, 'outerHTML'],
  [/\binsertAdjacentHTML\b/, 'insertAdjacentHTML'],
  [/\bdocument\s*\.\s*write(ln)?\b/, 'document.write'],
  [/\beval\s*\(/, 'eval('],
  [/\bnew\s+Function\b/, 'new Function'],
  [/\bsetHTMLUnsafe\b/, 'setHTMLUnsafe'],
  [/\bcreateContextualFragment\b/, 'createContextualFragment'],
  [/<[A-Za-z][^>]*\sstyle\s*=/, 'style attribute'],
  [/\bstyle:[a-z-]+/, 'style: directive'],
  [/\.setAttribute\(\s*['"`]style['"`]/, "setAttribute('style')"],
  [/<style[\s>]/i, '<style> block (use ui/src/app.css)'],
];
// Built from pieces so this file does not trip its own check.
const ACK = new RegExp('/' + 'ack', 'i');

const hits = [];
function walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (SKIP_DIRS.has(p)) continue;
    if (statSync(p).isDirectory()) { walk(p); continue; }
    if (!/\.(ts|js|svelte|css|html)$/.test(name)) continue;
    const isTest = /\.test\.ts$/.test(name) || name === 'test-setup.ts';
    const lines = readFileSync(p, 'utf8').split('\n');
    lines.forEach((line, i) => {
      const where = `${relative('.', p)}:${i + 1}`;
      if (ACK.test(line)) hits.push(`${where}: references the ack route`);
      if (isTest) return;
      for (const [re, label] of SINKS) if (re.test(line)) hits.push(`${where}: ${label}`);
    });
  }
}
walk(root);
if (hits.length) {
  console.error(hits.join('\n'));
  process.exit(1);
}
console.log(`lint-sinks: ok (${root})`);
