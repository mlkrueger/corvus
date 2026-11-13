// Message filtering (pure) and the query the dashboard sends for a filter.
import type { MessageListItem } from './types/MessageListItem';

export type MessageFilter = { mode: 'all' } | { mode: 'unread' } | { mode: 'agent'; agent: string };

export const ALL_MESSAGES: MessageFilter = { mode: 'all' };

export function isUnread(m: MessageListItem): boolean {
  return m.unread_by.length > 0;
}

export function filterMessages(messages: MessageListItem[], filter: MessageFilter): MessageListItem[] {
  const newestFirst = [...messages].sort((a, b) => b.id - a.id);
  switch (filter.mode) {
    case 'all':
      return newestFirst;
    case 'unread':
      return newestFirst.filter(isUnread);
    case 'agent':
      return newestFirst.filter((m) => m.sender === filter.agent || m.recipient === filter.agent);
  }
}

export const MESSAGE_LIMIT = 200;

/** The GET path for a filter; the server narrows too so old unread messages are not cut off by the limit. */
export function messagesPath(filter: MessageFilter): string {
  const q = new URLSearchParams({ limit: String(MESSAGE_LIMIT) });
  if (filter.mode === 'unread') q.set('unread_only', 'true');
  if (filter.mode === 'agent') q.set('agent', filter.agent);
  return `/api/messages?${q.toString()}`;
}

export function replyType(m: Pick<MessageListItem, 'type'>): 'answer' | 'note' {
  return m.type === 'question' ? 'answer' : 'note';
}

export function recipientLabel(m: Pick<MessageListItem, 'recipient'>): string {
  return m.recipient ?? 'everyone';
}
