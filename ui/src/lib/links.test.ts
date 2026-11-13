/* eslint-disable no-script-url -- hostile URLs are the test input */
import { describe, expect, it } from 'vitest';
import { safeHttpsUrl } from './links';

describe('safeHttpsUrl', () => {
  it('accepts https', () => {
    expect(safeHttpsUrl('https://example.atlassian.net/browse/AA-1')).toBe('https://example.atlassian.net/browse/AA-1');
  });
  it.each([
    'http://example.com',
    'javascript:alert(1)',
    'JaVaScRiPt:alert(1)',
    'data:text/html,<script>1</script>',
    'file:///etc/passwd',
    'ipc://localhost',
    '//example.com',
    'example.com',
    '',
    ' javascript:alert(1)',
  ])('rejects %j', (u) => {
    expect(safeHttpsUrl(u)).toBeNull();
  });
  it('rejects non-strings', () => {
    expect(safeHttpsUrl(null)).toBeNull();
    expect(safeHttpsUrl(undefined)).toBeNull();
    expect(safeHttpsUrl(42)).toBeNull();
  });
});
