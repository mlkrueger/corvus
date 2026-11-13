// Viewing never acks (system/05): no source file may reference the ack route.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? files(p) : [p];
  });
}

describe('viewing never acks', () => {
  it('no file under src references the ack route', () => {
    const route = new RegExp('/' + 'ack', 'i');
    const offenders = files(__dirname).filter((f) => route.test(readFileSync(f, 'utf8')));
    expect(offenders).toEqual([]);
  });

  it('no non-generated file calls an ack function either', () => {
    const call = new RegExp('\\back' + 'Message\\b|\\bAck' + 'Message\\b');
    const offenders = files(__dirname).filter(
      (f) => !f.includes(join('lib', 'types')) && !f.endsWith('no-ack.test.ts') && call.test(readFileSync(f, 'utf8')),
    );
    expect(offenders).toEqual([]);
  });
});
