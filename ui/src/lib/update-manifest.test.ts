import { describe, expect, it } from 'vitest';
import { manifest, platformKey } from '../../scripts/update-manifest.mjs';

const input = {
  version: '0.2.0',
  arch: 'arm64',
  url: 'https://github.com/o/r/releases/download/v0.2.0/Corvus-0.2.0-macos-arm64.app.tar.gz',
  signature: 'c2lnbmF0dXJl\n',
  pubDate: '2026-11-18T00:00:00.000Z',
};

describe('update manifest', () => {
  it('puts the version, URL and trimmed signature where the updater reads them', () => {
    expect(manifest(input)).toEqual({
      version: '0.2.0',
      pub_date: '2026-11-18T00:00:00.000Z',
      platforms: { 'darwin-aarch64': { signature: 'c2lnbmF0dXJl', url: input.url } },
    });
  });

  it('names the platform after the architecture', () => {
    expect(platformKey('arm64')).toBe('darwin-aarch64');
    expect(platformKey('aarch64')).toBe('darwin-aarch64');
    expect(platformKey('x86_64')).toBe('darwin-x86_64');
    expect(() => platformKey('riscv64')).toThrow(/unsupported/);
  });

  it('rejects a bad version, a non-https URL and an empty signature', () => {
    expect(() => manifest({ ...input, version: 'v0.2' })).toThrow(/not a version/);
    expect(() => manifest({ ...input, url: 'http://example.test/a.tar.gz' })).toThrow(/https/);
    expect(() => manifest({ ...input, signature: ' \n' })).toThrow(/empty/);
  });
});
