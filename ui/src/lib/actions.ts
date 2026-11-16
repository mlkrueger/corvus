// Operator actions. Each builds the request the HTTP API expects and sends a fresh idempotency key per call.
import { newKey, request } from './api';
import { replyType } from './messages';
import type { CancelTask } from './types/CancelTask';
import type { CreateThread } from './types/CreateThread';
import type { MessageListItem } from './types/MessageListItem';
import type { MessageType } from './types/MessageType';
import type { PostMessage } from './types/PostMessage';
import type { ReassignMail } from './types/ReassignMail';
import type { ReassignResult } from './types/ReassignResult';
import type { SendDirect } from './types/SendDirect';
import type { ThreadView } from './types/ThreadView';

export const BROADCAST_THREAD_KEY = 'operator-broadcast';
export const BROADCAST_THREAD_TITLE = 'Operator broadcasts';

export const MESSAGE_TYPES: MessageType[] = ['note', 'question', 'answer', 'handoff', 'status', 'decision'];

export async function sendDirect(operator: string, to: string, body: string, type: MessageType): Promise<void> {
  const req: SendDirect = { from: operator, to, body, type, idempotency_key: newKey(), session_id: null };
  await request('POST', '/api/messages/direct', req);
}

/**
 * Broadcast needs a thread: create or reuse the operator's keyed thread, then post without `to`.
 * `wake` also wakes idle sessions now (one model turn in each); without it the broadcast waits for each
 * session's next turn.
 */
export async function sendBroadcast(
  operator: string,
  body: string,
  type: MessageType,
  wake = false,
): Promise<void> {
  const threadReq: CreateThread = {
    creator: operator,
    title: BROADCAST_THREAD_TITLE,
    task_id: null,
    key: BROADCAST_THREAD_KEY,
    session_id: null,
  };
  const thread = await request<ThreadView>('POST', '/api/threads', threadReq);
  const req: PostMessage = {
    thread_id: thread.id,
    from: operator,
    body,
    type,
    to: null,
    in_reply_to: null,
    artifacts: [],
    idempotency_key: newKey(),
    session_id: null,
    wake,
  };
  await request('POST', '/api/messages', req);
}

/**
 * Operator action: move `from`'s unread direct mail to `to` (each message is copied; the original is marked
 * reassigned, which is not an acknowledgement).
 */
export async function reassignMail(from: string, to: string): Promise<ReassignResult> {
  const req: ReassignMail = { from, to };
  return request<ReassignResult>('POST', '/api/admin/reassign-mail', req);
}

/** Who a reply goes to: the original sender, or (for the operator's own message) its recipient. */
export function replyTarget(operator: string, m: Pick<MessageListItem, 'sender' | 'recipient'>): string | null {
  return m.sender === operator ? m.recipient : m.sender;
}

export async function sendReply(
  operator: string,
  m: MessageListItem,
  body: string,
  type: MessageType = replyType(m),
): Promise<void> {
  const req: PostMessage = {
    thread_id: m.thread_id,
    from: operator,
    body,
    type,
    to: replyTarget(operator, m),
    in_reply_to: m.id,
    artifacts: [],
    idempotency_key: newKey(),
    session_id: null,
    wake: false,
  };
  await request('POST', '/api/messages', req);
}

export async function cancelTask(operator: string, taskId: string, reason: string): Promise<void> {
  const req: CancelTask = {
    by: operator,
    reason: reason.trim() === '' ? null : reason.trim(),
    cascade: false,
    session_id: null,
  };
  await request('POST', `/api/tasks/${encodeURIComponent(taskId)}/cancel`, req);
}
