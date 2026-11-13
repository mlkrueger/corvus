import { describe, expect, it } from 'vitest';
import { message } from '../fixtures';
import { filterMessages, messagesPath, replyType } from './messages';

const msgs = [
  message({ id: 1, sender: 'alice', recipient: 'operator', unread_by: [] }),
  message({ id: 3, sender: 'operator', recipient: 'bob', unread_by: ['bob'] }),
  message({ id: 2, sender: 'carol', recipient: null, unread_by: ['alice', 'bob'] }),
];

describe('filterMessages', () => {
  it('all: newest first', () => {
    expect(filterMessages(msgs, { mode: 'all' }).map((m) => m.id)).toEqual([3, 2, 1]);
  });
  it('unread: only messages someone has not read', () => {
    expect(filterMessages(msgs, { mode: 'unread' }).map((m) => m.id)).toEqual([3, 2]);
  });
  it('agent: sender or recipient', () => {
    expect(filterMessages(msgs, { mode: 'agent', agent: 'bob' }).map((m) => m.id)).toEqual([3]);
    expect(filterMessages(msgs, { mode: 'agent', agent: 'alice' }).map((m) => m.id)).toEqual([1]);
    expect(filterMessages(msgs, { mode: 'agent', agent: 'nobody' })).toEqual([]);
  });
  it('does not mutate its input', () => {
    const copy = [...msgs];
    filterMessages(msgs, { mode: 'all' });
    expect(msgs).toEqual(copy);
  });
  it('empty list', () => {
    expect(filterMessages([], { mode: 'unread' })).toEqual([]);
  });
});

describe('messagesPath', () => {
  it('narrows server-side too', () => {
    expect(messagesPath({ mode: 'all' })).toBe('/api/messages?limit=200');
    expect(messagesPath({ mode: 'unread' })).toBe('/api/messages?limit=200&unread_only=true');
    expect(messagesPath({ mode: 'agent', agent: 'a b&c' })).toBe('/api/messages?limit=200&agent=a+b%26c');
  });
});

describe('replyType', () => {
  it('answers questions, notes otherwise', () => {
    expect(replyType({ type: 'question' })).toBe('answer');
    for (const t of ['note', 'answer', 'handoff', 'status', 'decision'] as const) expect(replyType({ type: t })).toBe('note');
  });
});
