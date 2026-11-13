import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Poller, type PollDoc } from './poll.svelte';

function fakeDoc(): PollDoc & { listeners: (() => void)[]; fire: () => void } {
  const listeners: (() => void)[] = [];
  return {
    hidden: false,
    listeners,
    addEventListener: (_t, fn) => void listeners.push(fn),
    removeEventListener: (_t, fn) => void listeners.splice(listeners.indexOf(fn), 1),
    fire() {
      listeners.forEach((l) => l());
    },
  };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('Poller', () => {
  it('ticks immediately and then every interval', async () => {
    const tick = vi.fn(async () => {});
    const p = new Poller(tick, 3000, fakeDoc());
    p.start();
    expect(tick).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(3000);
    expect(tick).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(6000);
    expect(tick).toHaveBeenCalledTimes(4);
    p.stop();
  });

  it('pause stops ticking and resume ticks at once', async () => {
    const tick = vi.fn(async () => {});
    const p = new Poller(tick, 3000, fakeDoc());
    p.start();
    p.pause();
    expect(p.paused).toBe(true);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(tick).toHaveBeenCalledTimes(1);
    p.resume();
    expect(tick).toHaveBeenCalledTimes(2);
    p.stop();
  });

  it('does not tick while the document is hidden and catches up when visible', async () => {
    const doc = fakeDoc();
    const tick = vi.fn(async () => {});
    const p = new Poller(tick, 3000, doc);
    p.start();
    doc.hidden = true;
    await vi.advanceTimersByTimeAsync(9000);
    expect(tick).toHaveBeenCalledTimes(1);
    doc.hidden = false;
    doc.fire();
    expect(tick).toHaveBeenCalledTimes(2);
    p.stop();
  });

  it('never overlaps: a tick is skipped while one is in flight', async () => {
    let release: () => void = () => {};
    const tick = vi.fn(() => new Promise<void>((r) => (release = r)));
    const p = new Poller(tick, 3000, fakeDoc());
    p.start();
    await vi.advanceTimersByTimeAsync(9000);
    expect(tick).toHaveBeenCalledTimes(1);
    release();
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(3000);
    expect(tick).toHaveBeenCalledTimes(2);
    p.stop();
  });

  it('survives a failing tick', async () => {
    const tick = vi.fn(async () => {
      throw new Error('boom');
    });
    const p = new Poller(tick, 1000, fakeDoc());
    p.start();
    await vi.advanceTimersByTimeAsync(3000);
    expect(tick).toHaveBeenCalledTimes(4);
    p.stop();
  });

  it('stop removes the timer and the listener', async () => {
    const doc = fakeDoc();
    const tick = vi.fn(async () => {});
    const p = new Poller(tick, 1000, doc);
    p.start();
    p.stop();
    await vi.advanceTimersByTimeAsync(5000);
    expect(tick).toHaveBeenCalledTimes(1);
    expect(doc.listeners).toHaveLength(0);
  });

  it('run(true) forces a tick while paused', async () => {
    const tick = vi.fn(async () => {});
    const p = new Poller(tick, 1000, fakeDoc());
    p.pause();
    expect(await p.run()).toBe(false);
    expect(await p.run(true)).toBe(true);
    expect(tick).toHaveBeenCalledTimes(1);
  });
});
