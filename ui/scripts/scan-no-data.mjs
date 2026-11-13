// "Bundle has no data": fails if any needle (a distinctive string from a seeded database) appears in a file under
// the built dist. Usage: node scripts/scan-no-data.mjs <dist dir> <needle>...   (also importable by e2e)
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export function findInDir(dir, needles) {
  const hits = [];
  const walk = (d) => {
    for (const name of readdirSync(d)) {
      const p = join(d, name);
      if (statSync(p).isDirectory()) { walk(p); continue; }
      const text = readFileSync(p, 'utf8');
      for (const n of needles) if (n && text.includes(n)) hits.push(`${p}: contains ${JSON.stringify(n)}`);
    }
  };
  walk(dir);
  return hits;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [dir = 'dist', ...needles] = process.argv.slice(2);
  const hits = findInDir(dir, needles);
  if (hits.length) {
    console.error(hits.join('\n'));
    process.exit(1);
  }
  console.log(`no-data scan: ok (${dir}, ${needles.length} needles)`);
}
