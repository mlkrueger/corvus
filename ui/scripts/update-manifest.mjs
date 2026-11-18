// Writes the update manifest (`latest.json`) the desktop app reads from the latest GitHub Release (D0027).
// Usage: node update-manifest.mjs <version> <arch> <archive-url> <signature-file>   (prints JSON)
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

/** The updater's platform key for a macOS build on `arch` (as `uname -m` reports it). */
export function platformKey(arch) {
  if (arch === 'arm64' || arch === 'aarch64') return 'darwin-aarch64';
  if (arch === 'x86_64') return 'darwin-x86_64';
  throw new Error(`unsupported architecture: ${arch}`);
}

/** The manifest for one macOS archive. `signature` is the content of the archive's `.sig` file. */
export function manifest({ version, arch, url, signature, pubDate }) {
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) throw new Error(`not a version: ${version}`);
  if (!url.startsWith('https://')) throw new Error(`the archive URL must be https: ${url}`);
  const sig = signature.trim();
  if (sig.length === 0) throw new Error('the signature is empty');
  return {
    version,
    pub_date: pubDate,
    platforms: { [platformKey(arch)]: { signature: sig, url } },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [version, arch, url, sigFile] = process.argv.slice(2);
  if (!version || !arch || !url || !sigFile) {
    console.error('usage: update-manifest.mjs <version> <arch> <archive-url> <signature-file>');
    process.exit(2);
  }
  const out = manifest({ version, arch, url, signature: readFileSync(sigFile, 'utf8'), pubDate: new Date().toISOString() });
  console.log(JSON.stringify(out, null, 2));
}
