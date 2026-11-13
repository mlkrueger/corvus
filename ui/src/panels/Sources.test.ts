import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { HOSTILE, NOW, fakeTransport, iso, sourceView, task, ticket } from '../fixtures';
import { setTransport } from '../lib/api';
import { SourcesState } from '../lib/sources.svelte';
import type { SourceId } from '../lib/types/SourceId';
import type { SourceView } from '../lib/types/SourceView';
import Sources from './Sources.svelte';

let restore: () => void = () => {};
afterEach(() => restore());

const err = (status: number, code: string, message: string) => () => ({ status, body: { error: { code, message } } });

interface Setup {
  jira?: SourceView | null;
  linear?: SourceView | null;
  extra?: Record<string, unknown>;
}

async function boot({ jira, linear, extra = {} }: Setup = {}) {
  const summaries = [
    { source: 'linear', enabled: linear !== undefined && linear !== null, site: null },
    { source: 'jira', enabled: jira !== undefined && jira !== null, site: null },
  ];
  const routes: Record<string, unknown> = { 'GET /api/sources': summaries, ...extra };
  if (jira) routes['GET /api/sources/jira'] = jira;
  if (linear) routes['GET /api/sources/linear'] = linear;
  const t = fakeTransport(routes);
  restore = setTransport(t.transport);
  const sources = new SourcesState();
  await sources.refresh();
  const r = render(Sources, { sources, operator: 'operator', now: NOW });
  return { t, sources, ...r };
}

const card = (key: string) => document.querySelector(`[data-ticket="${key}"]`) as HTMLElement;

describe('Sources panel', () => {
  it('is hidden entirely when no source is enabled', async () => {
    const { container } = await boot();
    expect(container.querySelector('section')).toBeNull();
    expect(screen.queryByText('Sources')).toBeNull();
  });

  it('has one tab per enabled source', async () => {
    await boot({ jira: sourceView('jira'), linear: sourceView('linear') });
    const tabs = screen.getAllByRole('tab');
    expect(tabs.map((t) => t.textContent)).toEqual(['Linear', 'Jira']);
    expect(tabs[0]).toHaveAttribute('aria-selected', 'true');
    await fireEvent.click(screen.getByRole('tab', { name: 'Jira' }));
    expect(screen.getByRole('tab', { name: 'Jira' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByLabelText(/Projects/)).toBeInTheDocument();
  });

  it('only shows the enabled source', async () => {
    await boot({ jira: sourceView('jira') });
    expect(screen.getAllByRole('tab')).toHaveLength(1);
    expect(screen.getByRole('tab', { name: 'Jira' })).toBeInTheDocument();
  });

  it('labels the project filter Projects for Jira and Teams for Linear', async () => {
    await boot({ jira: sourceView('jira'), linear: sourceView('linear') });
    expect(screen.getByLabelText(/^Teams/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/^Projects/)).toBeNull();
    await fireEvent.click(screen.getByRole('tab', { name: 'Jira' }));
    expect(screen.getByLabelText(/^Projects/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/^Teams/)).toBeNull();
  });

  it('renders status columns from the filter plus the done column, and the default Jira filter', async () => {
    await boot({
      jira: sourceView('jira', {
        tickets: [
          ticket({ key: 'AA-1', status: 'To Do' }),
          ticket({ key: 'AA-2', status: 'Done', status_category: 'done' }),
        ],
      }),
    });
    const heads = [...document.querySelectorAll('.source-board h3')].map((h) => h.textContent!.replace(/\s+/g, ' ').trim());
    expect(heads).toEqual(['Backlog 0', 'To Do 1', 'In Progress 0', 'Waiting for response 0', 'Done in last 7 days 1']);
    expect(within(document.querySelector('[data-column="source-done"]') as HTMLElement).getByText('AA-2')).toBeInTheDocument();
    // The Jira default filter shows in the form: statuses, window, and the AA rule.
    expect(screen.getByLabelText(/Statuses/)).toHaveValue('Backlog\nTo Do\nIn Progress\nWaiting for response');
    expect(screen.getByLabelText(/Done window/)).toHaveValue('7');
    expect(screen.getByLabelText(/Auto-queue rules/)).toHaveValue('AA: To Do, In Progress');
  });

  it('hides the done column when the window is 0', async () => {
    await boot({ jira: sourceView('jira', { filter: { statuses: ['To Do'], done_days: 0, projects: [], auto_queue: [] } }) });
    expect(document.querySelector('[data-column="source-done"]')).toBeNull();
  });

  it('links the key only for https urls; javascript: and http: render as text', async () => {
    await boot({
      jira: sourceView('jira', {
        tickets: [
          ticket({ key: 'AA-1', url: 'https://example.atlassian.net/browse/AA-1' }),
          ticket({ key: 'AA-2', url: 'java' + 'script:alert(1)' }),
          ticket({ key: 'AA-3', url: 'http://example.test/AA-3' }),
          ticket({ key: 'AA-4', url: '' }),
        ],
      }),
    });
    const link = within(card('AA-1')).getByRole('link', { name: 'AA-1' });
    expect(link).toHaveAttribute('href', 'https://example.atlassian.net/browse/AA-1');
    expect(link).toHaveAttribute('target', '_blank');
    expect(link.getAttribute('rel')).toBe('noopener noreferrer');
    for (const k of ['AA-2', 'AA-3', 'AA-4']) {
      expect(within(card(k)).queryByRole('link')).toBeNull();
      expect(within(card(k)).getByText(k)).toBeInTheDocument();
    }
  });

  it('shows auto, task state and by-hand badges', async () => {
    await boot({
      jira: sourceView('jira', {
        tickets: [
          ticket({ key: 'AA-1', auto_queue: true }),
          ticket({ key: 'AA-2', manual: true, task_id: 't-2', task_status: 'claimed' }),
          ticket({ key: 'AA-3', auto_queue: true, task_id: 't-3', task_status: 'open' }),
          ticket({ key: 'AA-4', task_id: 't-4', task_status: 'cancelled' }),
        ],
      }),
    });
    const badges = (k: string) => [...card(k).querySelectorAll('[data-badge]')].map((b) => b.textContent);
    expect(badges('AA-1')).toEqual(['auto']);
    expect(badges('AA-2')).toEqual(['claimed', 'by hand']);
    expect(badges('AA-3')).toEqual(['auto', 'open']);
    expect(badges('AA-4')).toEqual(['cancelled']);
  });

  it('offers Add to queue / Queue now / Remove from queue per ticket state', async () => {
    await boot({
      jira: sourceView('jira', {
        tickets: [
          ticket({ key: 'AA-1' }),
          ticket({ key: 'AA-2', auto_queue: true }),
          ticket({ key: 'AA-3', task_id: 't-3', task_status: 'open' }),
          ticket({ key: 'AA-4', task_id: 't-4', task_status: 'claimed' }),
          ticket({ key: 'AA-5', status: 'Done', status_category: 'done' }),
        ],
      }),
    });
    const names = (k: string) => within(card(k)).queryAllByRole('button').map((b) => b.textContent);
    expect(names('AA-1')).toEqual(['Add to queue']);
    expect(names('AA-2')).toEqual(['Queue now']);
    expect(names('AA-3')).toEqual(['Remove from queue']);
    expect(names('AA-4')).toEqual([]);
    expect(names('AA-5')).toEqual([]);
  });

  it('Add to queue posts {key} to the queue route and reloads the view', async () => {
    const { t } = await boot({
      jira: sourceView('jira', { tickets: [ticket({ key: 'AA-1' })] }),
      extra: { 'POST /api/sources/jira/queue': task({ id: 'new-1' }) },
    });
    await fireEvent.click(within(card('AA-1')).getByRole('button', { name: 'Add to queue' }));
    await waitFor(() => expect(screen.getByText('AA-1 queued')).toBeInTheDocument());
    const post = t.api().find((c) => c.method === 'POST');
    expect(post).toMatchObject({ path: '/api/sources/jira/queue', body: { key: 'AA-1' } });
    const gets = t.api().filter((c) => c.path === '/api/sources/jira');
    expect(gets).toHaveLength(2);
  });

  it('Remove from queue cancels the open task as the operator', async () => {
    const { t } = await boot({
      jira: sourceView('jira', { tickets: [ticket({ key: 'AA-3', task_id: 't-3', task_status: 'open' })] }),
      extra: { 'POST /api/tasks/t-3/cancel': task({ id: 't-3', status: 'cancelled' }) },
    });
    await fireEvent.click(within(card('AA-3')).getByRole('button', { name: 'Remove from queue' }));
    await waitFor(() => expect(screen.getByText('AA-3 removed from the queue')).toBeInTheDocument());
    const post = t.api().find((c) => c.method === 'POST')!;
    expect(post.path).toBe('/api/tasks/t-3/cancel');
    expect(post.body).toMatchObject({ by: 'operator', reason: 'removed from queue on the dashboard' });
  });

  it('shows a queue conflict inline', async () => {
    await boot({
      jira: sourceView('jira', { tickets: [ticket({ key: 'AA-1' })] }),
      extra: { 'POST /api/sources/jira/queue': err(409, 'conflict', 'AA-1 already has a live task') },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Add to queue' }));
    expect(await screen.findByText('conflict: AA-1 already has a live task')).toBeInTheDocument();
  });

  it('Sync now posts to the sync route and shows the result', async () => {
    const stats = { tickets: 4, tasks_created: 2, comments_delivered: 1, followups: 0, resumed: 0, out_of_scope: 0 };
    const { t } = await boot({
      jira: sourceView('jira'),
      extra: { 'POST /api/sources/jira/sync': { ok: true, stats, error: null } },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Sync now' }));
    expect(await screen.findByText('synced 4 tickets, 2 tasks created, 1 comments delivered')).toBeInTheDocument();
    expect(t.api().find((c) => c.method === 'POST')).toMatchObject({ path: '/api/sources/jira/sync' });
  });

  it('a failed sync result is shown as an error', async () => {
    await boot({
      jira: sourceView('jira'),
      extra: { 'POST /api/sources/jira/sync': { ok: false, stats: null, error: 'auth: 401 from Jira' } },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Sync now' }));
    const alert = await screen.findByText('sync failed: auth: 401 from Jira');
    expect(alert).toHaveClass('error');
  });

  it('shows the relative last-sync time, or last_error as an error', async () => {
    const ok = await boot({ jira: sourceView('jira') });
    expect(document.querySelector('[data-sync="last"]')).toHaveTextContent('last sync 2m 00s ago');
    ok.unmount();
    restore();
    await boot({ jira: sourceView('jira', { last_error: 'network: timed out', last_sync_at: iso(-30) }) });
    const e = document.querySelector('[data-sync="error"]')!;
    expect(e).toHaveTextContent('last sync failed: network: timed out');
    expect(e).toHaveClass('error');
  });

  it('renders hostile summaries, statuses and errors as literal text', async () => {
    await boot({
      jira: sourceView('jira', {
        filter: { statuses: [HOSTILE], done_days: 0, projects: [], auto_queue: [] },
        tickets: [ticket({ key: 'AA-1', summary: HOSTILE, status: HOSTILE })],
        last_error: HOSTILE,
        known_statuses: [HOSTILE + '2'],
      }),
    });
    expect(within(card('AA-1')).getByText(HOSTILE)).toBeInTheDocument();
    expect(document.querySelector('.source-board h3')).toHaveTextContent(HOSTILE);
    expect(document.querySelector('[data-sync="error"]')).toHaveTextContent(HOSTILE);
    expect(screen.getByRole('button', { name: HOSTILE + '2' })).toBeInTheDocument();
    expect(document.querySelectorAll('img, script')).toHaveLength(0);
    expect((window as unknown as { __pwned?: unknown }).__pwned).toBeUndefined();
  });

  describe('filter form', () => {
    it('Save sends the edited SourceFilter (rules text via sourceRules) and applies the returned view', async () => {
      const saved = sourceView('jira', {
        filter: { statuses: ['To Do', 'Review'], done_days: 3, projects: ['AA', 'LAI'], auto_queue: [{ projects: ['*'], statuses: ['Review'] }] },
      });
      const { t } = await boot({ jira: sourceView('jira'), extra: { 'PUT /api/sources/jira/filter': saved } });
      await fireEvent.input(screen.getByLabelText(/Statuses/), { target: { value: 'To Do\nReview' } });
      await fireEvent.input(screen.getByLabelText(/Done window/), { target: { value: '3' } });
      await fireEvent.input(screen.getByLabelText(/^Projects/), { target: { value: 'AA, LAI' } });
      await fireEvent.input(screen.getByLabelText(/Auto-queue rules/), { target: { value: '*: Review' } });
      await fireEvent.click(screen.getByRole('button', { name: 'Save filter' }));
      await waitFor(() => expect(screen.getByText('filter saved')).toBeInTheDocument());
      const put = t.api().find((c) => c.method === 'PUT')!;
      expect(put.path).toBe('/api/sources/jira/filter');
      expect(put.body).toEqual(saved.filter);
      // The returned view is shown (columns follow the saved filter).
      expect([...document.querySelectorAll('.source-board h3')].map((h) => h.textContent!.trim().replace(/\s+\d+$/, ''))).toEqual([
        'To Do',
        'Review',
        'Done in last 3 days',
      ]);
    });

    it('clicking a suggestion appends the status and the suggestion disappears', async () => {
      await boot({ jira: sourceView('jira') });
      expect(screen.queryByRole('button', { name: 'To Do' })).toBeNull();
      await fireEvent.click(screen.getByRole('button', { name: 'In Review' }));
      expect(screen.getByLabelText(/Statuses/)).toHaveValue('Backlog\nTo Do\nIn Progress\nWaiting for response\nIn Review');
      expect(screen.queryByRole('button', { name: 'In Review' })).toBeNull();
      expect(screen.getByRole('button', { name: 'Done' })).toBeInTheDocument();
    });

    it('shows per-line rule parse errors and sends nothing', async () => {
      const { t } = await boot({ jira: sourceView('jira') });
      await fireEvent.input(screen.getByLabelText(/Auto-queue rules/), { target: { value: 'AA: To Do\nbroken line' } });
      await fireEvent.click(screen.getByRole('button', { name: 'Save filter' }));
      expect(await screen.findByText('Rule line 2: expected "PROJECTS: STATUSES"')).toBeInTheDocument();
      expect(t.api().some((c) => c.method === 'PUT')).toBe(false);
    });

    it('shows a 422 message inline and keeps the edit', async () => {
      await boot({
        jira: sourceView('jira'),
        extra: { 'PUT /api/sources/jira/filter': err(422, 'validation', 'done_days must be 0-90') },
      });
      await fireEvent.input(screen.getByLabelText(/Done window/), { target: { value: '99' } });
      await fireEvent.click(screen.getByRole('button', { name: 'Save filter' }));
      expect(await screen.findByText('validation: done_days must be 0-90')).toBeInTheDocument();
      expect(screen.getByLabelText(/Done window/)).toHaveValue('99');
    });

    it('a poll does not overwrite an edit in progress', async () => {
      const { sources } = await boot({ jira: sourceView('jira') });
      await fireEvent.input(screen.getByLabelText(/Statuses/), { target: { value: 'Mine' } });
      await sources.reload('jira' as SourceId);
      expect(screen.getByLabelText(/Statuses/)).toHaveValue('Mine');
    });
  });
});
