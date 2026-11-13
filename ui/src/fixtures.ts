// Seeded data for unit tests, plus a recording fake transport.
import { vi } from 'vitest';
import type { Transport } from './lib/api';
import type { MessageListItem } from './lib/types/MessageListItem';
import type { Overview } from './lib/types/Overview';
import type { OverviewSession } from './lib/types/OverviewSession';
import type { SourceId } from './lib/types/SourceId';
import type { SourceTicketView } from './lib/types/SourceTicketView';
import type { SourceView } from './lib/types/SourceView';
import type { TaskView } from './lib/types/TaskView';

export const NOW = Date.parse('2026-11-13T12:00:00.000Z');
export const iso = (offsetSeconds: number): string => new Date(NOW + offsetSeconds * 1000).toISOString();
export const HOSTILE = '<img src=x onerror=window.__pwned=1>';

export function task(over: Partial<TaskView> = {}): TaskView {
  return {
    id: '01a1d68c-0000-7000-8000-000000000001',
    title: 'Write the parser',
    body: '',
    priority: 2,
    status: 'open',
    pool: 'default',
    recipient_agent: null,
    creator: 'operator',
    result_summary: null,
    cancel_reason: null,
    cancelled_by: null,
    created_at: iso(-600),
    updated_at: iso(-600),
    completed_at: null,
    cancelled_at: null,
    thread_id: null,
    depends_on: [],
    artifacts: [],
    blocked: false,
    blocked_by_cancelled: false,
    holder: null,
    ...over,
  };
}

export function session(over: Partial<OverviewSession> = {}): OverviewSession {
  return {
    unread: 0,
    id: 's-1',
    agent_name: 'alice',
    client_name: 'claude',
    metadata: {},
    name_generated: false,
    started_at: iso(-3600),
    last_seen_at: iso(-30),
    ended_at: null,
    end_reason: null,
    presence: 'active',
    current_task: null,
    lease_expires_at: null,
    ...over,
  };
}

export function message(over: Partial<MessageListItem> = {}): MessageListItem {
  return {
    thread_title: 'Direct: alice',
    read_by: [],
    unread_by: [],
    id: 1,
    thread_id: 't-1',
    sender: 'alice',
    recipient: 'operator',
    type: 'note',
    in_reply_to: null,
    body: 'hello',
    session_id: null,
    created_at: iso(-60),
    artifacts: [],
    ...over,
  };
}

export function overview(over: Partial<Overview> = {}): Overview {
  const cancelled = task({
    id: 'c-1',
    title: 'Dropped idea',
    status: 'cancelled',
    cancel_reason: 'no longer needed',
  });
  const blocked = task({ id: 'b-1', title: 'Depends on dropped', blocked: true, blocked_by_cancelled: true });
  const claimed = task({
    id: 'k-1',
    title: 'Index the repo',
    status: 'claimed',
    priority: 5,
    holder: {
      agent_name: 'alice',
      session_id: 's-1',
      claim_id: 'cl-1',
      claimed_at: iso(-100),
      lease_expires_at: iso(600),
    },
  });
  return {
    generated_at: iso(0),
    operator_name: 'operator',
    thresholds: { active_seconds: 120, idle_seconds: 900, lease_seconds: 900, auto_end_seconds: 3600 },
    presence_counts: { working: 1, active: 1, idle: 0, offline: 1 },
    sessions: [
      session({ id: 's-1', agent_name: 'alice', presence: 'working', unread: 2, current_task: 'k-1', lease_expires_at: iso(600) }),
      session({ id: 's-2', agent_name: 'bob', client_name: 'codex', presence: 'active' }),
      session({ id: 's-3', agent_name: 'carol', presence: 'offline', last_seen_at: iso(-7200) }),
      session({ id: 's-4', agent_name: 'dave', presence: 'ended', ended_at: iso(-300), end_reason: 'ended' }),
    ],
    tasks: {
      open: [task({ id: 'o-1', title: 'Write the parser', priority: 1 }), blocked],
      claimed: [claimed],
      done: [task({ id: 'd-1', title: 'Ship it', status: 'done', result_summary: 'shipped' })],
      cancelled: [cancelled],
    },
    stats: {
      open_tasks: 2,
      claimed_tasks: 1,
      active_leases: 1,
      expired_leases: 1,
      unread_total: 3,
      unread_by_agent: { alice: 2, bob: 1 },
    },
    blocked_by_cancelled: 1,
    events: [
      {
        id: 7,
        kind: 'task.claimed',
        summary: 'alice claimed Index the repo',
        agent_name: 'alice',
        session_id: 's-1',
        task_id: 'k-1',
        thread_id: null,
        message_id: null,
        created_at: iso(-100),
      },
    ],
    decisions: [
      {
        id: 'dec-1',
        title: 'Use SQLite',
        decision: 'We use SQLite.',
        rationale: 'Local-first.',
        decided_by: 'operator',
        task_id: null,
        thread_id: null,
        supersedes: null,
        superseded_by: null,
        created_at: iso(-900),
      },
    ],
    ...over,
  };
}

export function ticket(over: Partial<SourceTicketView> = {}): SourceTicketView {
  return {
    key: 'AA-1',
    project: 'AA',
    summary: 'Fix the login bug',
    status: 'To Do',
    status_category: 'todo',
    priority: 2,
    url: 'https://example.atlassian.net/browse/AA-1',
    resolved_at: null,
    assignee: 'me',
    auto_queue: false,
    manual: false,
    task_id: null,
    task_status: null,
    ...over,
  };
}

/** The Jira default filter (system/07a): four statuses, 7 days, all projects, one rule for project AA. */
export function sourceView(source: SourceId = 'jira', over: Partial<SourceView> = {}): SourceView {
  return {
    source,
    enabled: true,
    site: source === 'jira' ? 'example.atlassian.net' : 'linear.app',
    filter: {
      statuses: ['Backlog', 'To Do', 'In Progress', 'Waiting for response'],
      done_days: 7,
      projects: [],
      auto_queue: [{ projects: ['AA'], statuses: ['To Do', 'In Progress'] }],
    },
    tickets: [],
    known_statuses: ['Backlog', 'To Do', 'In Progress', 'In Review', 'Waiting for response', 'Done'],
    last_sync_at: iso(-120),
    last_stats: { tickets: 0, tasks_created: 0, comments_delivered: 0, followups: 0, resumed: 0, out_of_scope: 0 },
    last_error: null,
    ...over,
  };
}

export interface Call {
  cmd: string;
  args?: Record<string, unknown>;
  method?: string;
  path?: string;
  body?: unknown;
}

type Handler = (call: Call) => { status: number; body: unknown };

/** A fake transport: `routes` maps "METHOD /path-prefix" to a response (or handler). Records every call. */
export function fakeTransport(routes: Record<string, unknown | Handler> = {}) {
  const calls: Call[] = [];
  const transport: Transport = vi.fn(async (cmd, args) => {
    const call: Call = { cmd, args };
    if (cmd === 'api') {
      call.method = args?.method as string;
      call.path = args?.path as string;
      call.body = args?.body;
    }
    calls.push(call);
    if (cmd !== 'api') {
      const r = routes[cmd];
      if (r === undefined) throw new Error(`unknown command ${cmd}`);
      return typeof r === 'function' ? (r as Handler)(call) : r;
    }
    const key = Object.keys(routes)
      .filter((k) => `${call.method} ${call.path}`.startsWith(k))
      .sort((a, b) => b.length - a.length)[0];
    if (key === undefined) return { status: 404, body: { error: { code: 'not_found', message: 'no route' } } };
    const r = routes[key];
    if (typeof r === 'function') return (r as Handler)(call);
    return { status: 200, body: r };
  });
  return { transport, calls, api: () => calls.filter((c) => c.cmd === 'api') };
}
