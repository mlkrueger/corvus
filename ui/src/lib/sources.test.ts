import { describe, expect, it } from 'vitest';
import { sourceView, ticket } from '../fixtures';
import {
  appendStatus,
  boardColumns,
  canQueue,
  canRemove,
  filterFromForm,
  formFromFilter,
  projectsLabel,
  queueLabel,
  suggestions,
} from './sources';

describe('boardColumns', () => {
  it('has a column per filter status plus a done column', () => {
    const v = sourceView('jira', {
      tickets: [
        ticket({ key: 'AA-1', status: 'To Do' }),
        ticket({ key: 'AA-2', status: 'in progress', status_category: 'in_progress' }),
        ticket({ key: 'AA-3', status: 'Done', status_category: 'done' }),
      ],
    });
    const cols = boardColumns(v);
    expect(cols.map((c) => c.title)).toEqual(['Backlog', 'To Do', 'In Progress', 'Waiting for response', 'Done in last 7 days']);
    expect(cols.map((c) => c.tickets.map((t) => t.key))).toEqual([[], ['AA-1'], ['AA-2'], [], ['AA-3']]);
  });

  it('hides the done column when done_days is 0 and keeps done tickets in their status column', () => {
    const v = sourceView('jira', {
      filter: { statuses: ['Done'], done_days: 0, projects: [], auto_queue: [] },
      tickets: [ticket({ key: 'AA-3', status: 'Done', status_category: 'done' })],
    });
    const cols = boardColumns(v);
    expect(cols.map((c) => c.title)).toEqual(['Done']);
    expect(cols[0]!.tickets).toHaveLength(1);
  });

  it('has no columns without a filter', () => {
    expect(boardColumns(sourceView('jira', { filter: null }))).toEqual([]);
  });
});

describe('queue rules', () => {
  it('queues tickets without a live task that are not done', () => {
    expect(canQueue(ticket())).toBe(true);
    expect(canQueue(ticket({ task_id: 't', task_status: 'cancelled' }))).toBe(true);
    expect(canQueue(ticket({ task_id: 't', task_status: 'open' }))).toBe(false);
    expect(canQueue(ticket({ task_id: 't', task_status: 'claimed' }))).toBe(false);
    expect(canQueue(ticket({ status_category: 'done' }))).toBe(false);
  });

  it('labels Queue now only for auto tickets with no task yet', () => {
    expect(queueLabel(ticket({ auto_queue: true }))).toBe('Queue now');
    expect(queueLabel(ticket({ auto_queue: true, task_id: 't', task_status: 'done' }))).toBe('Add to queue');
    expect(queueLabel(ticket())).toBe('Add to queue');
  });

  it('removes only while the task is open', () => {
    expect(canRemove(ticket({ task_id: 't', task_status: 'open' }))).toBe(true);
    expect(canRemove(ticket({ task_id: 't', task_status: 'claimed' }))).toBe(false);
    expect(canRemove(ticket())).toBe(false);
  });
});

describe('filter form', () => {
  it('round-trips the Jira default filter', () => {
    const f = sourceView('jira').filter!;
    const form = formFromFilter(f);
    expect(form.rules).toBe('AA: To Do, In Progress');
    const back = filterFromForm(form);
    expect(back).toEqual({ ok: true, filter: f });
  });

  it('parses multi-project rules, wildcards and comma-separated projects', () => {
    const r = filterFromForm({ statuses: ' A \n\nB\n', doneDays: '3', projects: 'AA, LAI ,', rules: 'AA, LAI: To Do, In Progress\n*: Open' });
    expect(r).toEqual({
      ok: true,
      filter: {
        statuses: ['A', 'B'],
        done_days: 3,
        projects: ['AA', 'LAI'],
        auto_queue: [
          { projects: ['AA', 'LAI'], statuses: ['To Do', 'In Progress'] },
          { projects: ['*'], statuses: ['Open'] },
        ],
      },
    });
  });

  it('reports per-line rule errors and a bad done window', () => {
    const r = filterFromForm({ statuses: 'A', doneDays: 'x', projects: '', rules: 'AA: To Do\nnonsense\n: X' });
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.errors).toEqual([
        'Done window must be a whole number of days.',
        'Rule line 2: expected "PROJECTS: STATUSES"',
        'Rule line 3: no projects before ":" (use * for any)',
      ]);
    }
  });

  it('suggests known statuses not yet listed and appends them', () => {
    expect(suggestions(['A', 'B', 'b', 'C'], 'a\nB')).toEqual(['C']);
    expect(appendStatus('A\nB\n', 'C')).toBe('A\nB\nC');
    expect(appendStatus('', 'C')).toBe('C');
  });

  it('uses the tracker vocabulary', () => {
    expect(projectsLabel('jira')).toBe('Projects');
    expect(projectsLabel('linear')).toBe('Teams');
  });
});
