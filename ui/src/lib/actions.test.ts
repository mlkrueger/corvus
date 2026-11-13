import { afterEach, describe, expect, it } from 'vitest';
import { fakeTransport, message } from '../fixtures';
import { BROADCAST_THREAD_KEY, cancelTask, replyTarget, sendBroadcast, sendDirect, sendReply } from './actions';
import { setTransport } from './api';

let restore: () => void = () => {};
afterEach(() => restore());

function setup() {
  const t = fakeTransport({
    'POST /api/messages': { id: 1 },
    'POST /api/messages/direct': { id: 2 },
    'POST /api/threads': { id: 'thread-b' },
    'POST /api/tasks/': { id: 'x' },
  });
  restore = setTransport(t.transport);
  return t;
}

describe('operator actions', () => {
  it('direct message: from the operator to one agent, with a fresh idempotency key each time', async () => {
    const t = setup();
    await sendDirect('op', 'alice', 'hi', 'note');
    await sendDirect('op', 'alice', 'hi', 'note');
    const [a, b] = t.api();
    expect(a).toMatchObject({ method: 'POST', path: '/api/messages/direct', body: { from: 'op', to: 'alice', body: 'hi', type: 'note' } });
    const ka = (a!.body as { idempotency_key: string }).idempotency_key;
    const kb = (b!.body as { idempotency_key: string }).idempotency_key;
    expect(ka).toBeTruthy();
    expect(ka).not.toBe(kb);
  });

  it('broadcast: creates/reuses the keyed thread, then posts without `to`', async () => {
    const t = setup();
    await sendBroadcast('op', 'all hands', 'status');
    const [thread, post] = t.api();
    expect(thread).toMatchObject({
      method: 'POST',
      path: '/api/threads',
      body: { creator: 'op', title: 'Operator broadcasts', key: BROADCAST_THREAD_KEY },
    });
    expect(post).toMatchObject({
      method: 'POST',
      path: '/api/messages',
      body: { thread_id: 'thread-b', from: 'op', body: 'all hands', type: 'status', to: null, in_reply_to: null },
    });
    expect((post!.body as { idempotency_key: string }).idempotency_key).toBeTruthy();
  });

  it('reply to a question defaults to type answer, in the same thread with in_reply_to', async () => {
    const t = setup();
    await sendReply('op', message({ id: 9, type: 'question', thread_id: 'th-9', sender: 'alice' }), 'yes');
    expect(t.api()[0]).toMatchObject({
      path: '/api/messages',
      body: { thread_id: 'th-9', in_reply_to: 9, type: 'answer', from: 'op', to: 'alice', body: 'yes' },
    });
  });

  it('reply to anything else defaults to note; an explicit type wins', async () => {
    const t = setup();
    await sendReply('op', message({ id: 4, type: 'status' }), 'ok');
    await sendReply('op', message({ id: 4, type: 'note' }), 'ok', 'handoff');
    expect(t.api().map((c) => (c.body as { type: string }).type)).toEqual(['note', 'handoff']);
  });

  it("reply to the operator's own message goes to its recipient", () => {
    expect(replyTarget('op', { sender: 'op', recipient: 'bob' })).toBe('bob');
    expect(replyTarget('op', { sender: 'op', recipient: null })).toBeNull();
    expect(replyTarget('op', { sender: 'alice', recipient: null })).toBe('alice');
  });

  it('cancel: by the operator with a reason, path-encoded id', async () => {
    const t = setup();
    await cancelTask('op', 'abc/1', ' not needed ');
    await cancelTask('op', 'abc', '  ');
    expect(t.api()[0]).toMatchObject({
      method: 'POST',
      path: '/api/tasks/abc%2F1/cancel',
      body: { by: 'op', reason: 'not needed', cascade: false },
    });
    expect((t.api()[1]!.body as { reason: unknown }).reason).toBeNull();
  });
});
