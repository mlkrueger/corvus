// Sources panel in a real browser engine. The IPC shim answers /api/sources* from canned in-test data
// (window.__sourcesMock, see harness.ts); every other request goes to the real throwaway `corvus serve`.
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import type { Page } from '@playwright/test';
import { UI_DIR } from './harness';
import { HOSTILE_IMG, expect, open, test } from './fixtures';

const SHOTS = join(UI_DIR, 'e2e', 'screenshots');

interface Req {
  method: string;
  path: string;
  body: unknown;
}

const STATS = { tickets: 6, tasks_created: 1, comments_delivered: 0, followups: 0, resumed: 0, out_of_scope: 0 };

function ticket(over: Record<string, unknown>) {
  return {
    key: 'AA-0',
    project: 'AA',
    summary: 'summary',
    status: 'To Do',
    status_category: 'todo',
    priority: 2,
    url: '',
    resolved_at: null,
    assignee: 'me',
    auto_queue: false,
    manual: false,
    task_id: null,
    task_status: null,
    ...over,
  };
}

function jiraView(openTaskId: string, over: Record<string, unknown> = {}) {
  return {
    source: 'jira',
    enabled: true,
    site: 'example.atlassian.net',
    filter: {
      statuses: ['Backlog', 'To Do', 'In Progress', 'Waiting for response'],
      done_days: 7,
      projects: [],
      auto_queue: [{ projects: ['AA'], statuses: ['To Do', 'In Progress'] }],
    },
    tickets: [
      ticket({ key: 'AA-1', summary: 'Auto ticket, not queued yet', auto_queue: true, url: 'https://example.atlassian.net/browse/AA-1' }),
      ticket({ key: 'AA-2', summary: 'Queued by hand', status: 'In Progress', status_category: 'in_progress', manual: true, task_id: openTaskId, task_status: 'open' }),
      ticket({ key: 'AA-3', summary: 'Plain ticket', status: 'Backlog', url: 'http://insecure.example/AA-3' }),
      ticket({ key: 'AA-4', summary: 'Shipped', status: 'Done', status_category: 'done', resolved_at: '2026-11-12T10:00:00.000Z', task_id: 'gone', task_status: 'done' }),
      ticket({ key: 'AA-5', summary: HOSTILE_IMG, status: 'To Do' }),
      ticket({ key: 'AA-6', summary: 'Bad link', status: 'To Do', url: 'java' + 'script:window.__pwned=1' }),
    ],
    known_statuses: ['Backlog', 'To Do', 'In Progress', 'In Review', 'Waiting for response', 'Done'],
    last_sync_at: new Date(Date.now() - 125_000).toISOString(),
    last_stats: STATS,
    last_error: null,
    ...over,
  };
}

async function mockSources(page: Page, mock: Record<string, unknown>): Promise<void> {
  await page.addInitScript((m) => {
    (window as unknown as { __sourcesMock: unknown }).__sourcesMock = m;
  }, mock);
}

const sourceRequests = (page: Page) =>
  page.evaluate(() => ((window as unknown as { __sourceRequests?: Req[] }).__sourceRequests ?? []));
const mutations = async (page: Page) => (await sourceRequests(page)).filter((r) => r.method !== 'GET');

const summaries = (jira: boolean, linear = false) => [
  { source: 'linear', enabled: linear, site: null },
  { source: 'jira', enabled: jira, site: jira ? 'example.atlassian.net' : null },
];

test.describe('sources panel in a browser engine', () => {
  test('is absent when GET /api/sources reports nothing enabled', async ({ page, h, probe }) => {
    await mockSources(page, { summaries: summaries(false), views: {} });
    await open(page, h);
    await expect(page.getByText('Agents & sessions')).toBeVisible();
    await expect.poll(async () => (await sourceRequests(page)).length).toBeGreaterThan(0);
    await expect(page.getByRole('heading', { name: 'Sources' })).toHaveCount(0);
    expect((await sourceRequests(page)).map((r) => r.path)).toEqual(['/api/sources']);
    expect(probe.consoleErrors).toEqual([]);
  });

  test('is absent against the real server when no source is configured', async ({ page, h }) => {
    await open(page, h);
    await expect.poll(() => h.requests.some((r) => r.path === '/api/sources')).toBe(true);
    await expect(page.getByRole('heading', { name: 'Sources' })).toHaveCount(0);
  });

  test('renders columns, cards and badges; links only https; hostile summary is literal', async ({ page, h, probe }, info) => {
    await mockSources(page, { summaries: summaries(true), views: { jira: jiraView('t-open') } });
    await open(page, h);
    await expect(page.getByRole('heading', { name: 'Sources' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Jira' })).toHaveAttribute('aria-selected', 'true');
    await expect(page.getByRole('tab')).toHaveCount(1);

    const heads = page.locator('.source-board h3');
    await expect(heads).toHaveCount(5);
    await expect(heads.nth(4)).toContainText('Done in last 7 days');
    await expect(page.locator('[data-column="source-done"] [data-ticket="AA-4"]')).toBeVisible();

    const card = (k: string) => page.locator(`[data-ticket="${k}"]`);
    await expect(card('AA-1').locator('[data-badge="auto"]')).toHaveText('auto');
    await expect(card('AA-1').getByRole('button', { name: 'Queue now' })).toBeVisible();
    await expect(card('AA-2').locator('[data-badge="task"]')).toHaveText('open');
    await expect(card('AA-2').locator('[data-badge="manual"]')).toHaveText('by hand');
    await expect(card('AA-2').getByRole('button', { name: 'Remove from queue' })).toBeVisible();
    await expect(card('AA-3').getByRole('button', { name: 'Add to queue' })).toBeVisible();
    await expect(card('AA-4').locator('[data-badge="task"]')).toHaveText('done');
    await expect(card('AA-4').getByRole('button')).toHaveCount(0);
    await expect(page.locator('[data-sync="last"]')).toContainText('last sync 2m');

    // Links: https only, noopener; http: and javascript: keys are text.
    const link = card('AA-1').getByRole('link', { name: 'AA-1' });
    await expect(link).toHaveAttribute('href', 'https://example.atlassian.net/browse/AA-1');
    await expect(link).toHaveAttribute('rel', 'noopener noreferrer');
    await expect(card('AA-3').getByRole('link')).toHaveCount(0);
    await expect(card('AA-6').getByRole('link')).toHaveCount(0);

    // Hostile summary: literal text, nothing executes, no CSP violation.
    await expect(card('AA-5').locator('.ticket-summary')).toHaveText(HOSTILE_IMG);
    await page.waitForTimeout(800);
    expect(await page.evaluate(() => (window as unknown as { __pwned?: unknown }).__pwned)).toBeUndefined();
    expect(await page.locator('#app img').count()).toBe(0);
    expect(await probe.violations()).toEqual([]);
    expect(probe.consoleErrors).toEqual([]);
    expect(await mutations(page)).toEqual([]);

    mkdirSync(SHOTS, { recursive: true });
    const panel = page.locator('.area-sources');
    await page.emulateMedia({ colorScheme: 'light' });
    await panel.screenshot({ path: join(SHOTS, `sources-light-${info.project.name}.png`) });
    await page.emulateMedia({ colorScheme: 'dark' });
    await panel.screenshot({ path: join(SHOTS, `sources-dark-${info.project.name}.png`) });
  });

  test('Add to queue / Queue now send the queue request for that key', async ({ page, h }) => {
    await mockSources(page, {
      summaries: summaries(true),
      views: { jira: jiraView('t-open') },
      responses: { 'POST /api/sources/jira/queue': { status: 200, body: { id: 'new-task' } } },
    });
    await open(page, h);
    await page.locator('[data-ticket="AA-3"]').getByRole('button', { name: 'Add to queue' }).click();
    await expect(page.getByText('AA-3 queued')).toBeVisible();
    await page.locator('[data-ticket="AA-1"]').getByRole('button', { name: 'Queue now' }).click();
    await expect(page.getByText('AA-1 queued')).toBeVisible();
    expect(await mutations(page)).toEqual([
      { method: 'POST', path: '/api/sources/jira/queue', body: { key: 'AA-3' } },
      { method: 'POST', path: '/api/sources/jira/queue', body: { key: 'AA-1' } },
    ]);
  });

  test('a queue conflict shows inline', async ({ page, h }) => {
    await mockSources(page, {
      summaries: summaries(true),
      views: { jira: jiraView('t-open') },
      responses: { 'POST /api/sources/jira/queue': { status: 409, body: { error: { code: 'conflict', message: 'AA-3 already has a live task' } } } },
    });
    await open(page, h);
    await page.locator('[data-ticket="AA-3"]').getByRole('button', { name: 'Add to queue' }).click();
    await expect(page.getByRole('alert').filter({ hasText: 'conflict: AA-3 already has a live task' })).toBeVisible();
  });

  test('Remove from queue cancels the open task through the real server', async ({ page, h }) => {
    const created = h.cliJson<{ id: string }>('task', 'create', '--title', '[AA-2] Queued by hand');
    await mockSources(page, { summaries: summaries(true), views: { jira: jiraView(created.id) } });
    await open(page, h);
    h.resetRequests();
    await page.locator('[data-ticket="AA-2"]').getByRole('button', { name: 'Remove from queue' }).click();
    await expect(page.getByText('AA-2 removed from the queue')).toBeVisible();
    expect(h.requests.filter((r) => r.method === 'POST').map((r) => r.path)).toEqual([`/api/tasks/${created.id}/cancel`]);
    const t = await h.api<{ status: string; cancel_reason: string; cancelled_by: string }>('GET', `/api/tasks/${created.id}`);
    expect(t).toMatchObject({ status: 'cancelled', cancel_reason: 'removed from queue on the dashboard', cancelled_by: 'operator' });
  });

  test('Sync now posts the sync request and shows the stats; a failed pass shows an error', async ({ page, h }) => {
    await mockSources(page, {
      summaries: summaries(true),
      views: { jira: jiraView('t-open') },
      responses: { 'POST /api/sources/jira/sync': { status: 200, body: { ok: true, stats: STATS, error: null } } },
    });
    await open(page, h);
    await page.getByRole('button', { name: 'Sync now' }).click();
    await expect(page.getByText('synced 6 tickets, 1 tasks created, 0 comments delivered')).toBeVisible();
    expect(await mutations(page)).toEqual([{ method: 'POST', path: '/api/sources/jira/sync', body: null }]);
  });

  test('a failed sync pass and last_error render as errors', async ({ page, h }, info) => {
    await mockSources(page, {
      summaries: summaries(true),
      views: { jira: jiraView('t-open', { last_error: 'auth: Jira rejected the credentials' }) },
      responses: { 'POST /api/sources/jira/sync': { status: 200, body: { ok: false, stats: null, error: 'auth: Jira rejected the credentials' } } },
    });
    await open(page, h);
    const err = page.locator('[data-sync="error"]');
    await expect(err).toHaveText('last sync failed: auth: Jira rejected the credentials');
    expect(await err.evaluate((e) => getComputedStyle(e).color)).not.toBe(await page.locator('[data-sync="last"]').evaluate((e) => getComputedStyle(e).color));
    await page.getByRole('button', { name: 'Sync now' }).click();
    await expect(page.locator('[data-notice="error"]')).toHaveText('sync failed: auth: Jira rejected the credentials');
    mkdirSync(SHOTS, { recursive: true });
    await page.locator('.area-sources').screenshot({ path: join(SHOTS, `sources-error-${info.project.name}.png`) });
  });

  test('filter form: suggestion click, save round trip, parse and 422 errors', async ({ page, h }) => {
    await mockSources(page, { summaries: summaries(true), views: { jira: jiraView('t-open') } });
    await open(page, h);
    const statuses = page.getByLabel(/^Statuses/);
    await expect(statuses).toHaveValue('Backlog\nTo Do\nIn Progress\nWaiting for response');
    await expect(page.getByLabel(/^Auto-queue rules/)).toHaveValue('AA: To Do, In Progress');
    await expect(page.getByLabel(/^Projects/)).toHaveValue('');

    await page.getByRole('button', { name: 'In Review' }).click();
    await expect(statuses).toHaveValue('Backlog\nTo Do\nIn Progress\nWaiting for response\nIn Review');
    await expect(page.getByRole('button', { name: 'In Review' })).toHaveCount(0);
    await page.getByLabel(/^Done window/).fill('3');
    await page.getByLabel(/^Projects/).fill('AA, LAI');
    await page.getByLabel(/^Auto-queue rules/).fill('AA, LAI: To Do, In Progress\n*: In Review');

    // A rule line that does not parse blocks the save and is reported per line.
    await page.getByLabel(/^Auto-queue rules/).fill('AA: To Do\nnot a rule');
    await page.getByRole('button', { name: 'Save filter' }).click();
    await expect(page.getByRole('alert').filter({ hasText: 'Rule line 2' })).toBeVisible();
    expect(await mutations(page)).toEqual([]);

    await page.getByLabel(/^Auto-queue rules/).fill('AA, LAI: To Do, In Progress\n*: In Review');
    await page.getByRole('button', { name: 'Save filter' }).click();
    await expect(page.getByText('filter saved')).toBeVisible();
    expect(await mutations(page)).toEqual([
      {
        method: 'PUT',
        path: '/api/sources/jira/filter',
        body: {
          statuses: ['Backlog', 'To Do', 'In Progress', 'Waiting for response', 'In Review'],
          done_days: 3,
          projects: ['AA', 'LAI'],
          auto_queue: [
            { projects: ['AA', 'LAI'], statuses: ['To Do', 'In Progress'] },
            { projects: ['*'], statuses: ['In Review'] },
          ],
        },
      },
    ]);
    // The saved filter drives the board and round-trips back into the form.
    await expect(page.locator('.source-board h3')).toHaveCount(6);
    await expect(page.locator('.source-board h3').last()).toContainText('Done in last 3 days');
    await expect(page.getByLabel(/^Auto-queue rules/)).toHaveValue('AA, LAI: To Do, In Progress\n*: In Review');
    await expect(page.getByLabel(/^Projects/)).toHaveValue('AA, LAI');
  });

  test('a 422 on save shows the server message inline', async ({ page, h }) => {
    await mockSources(page, {
      summaries: summaries(true),
      views: { jira: jiraView('t-open') },
      responses: { 'PUT /api/sources/jira/filter': { status: 422, body: { error: { code: 'validation', message: 'done_days must be 0-90' } } } },
    });
    await open(page, h);
    await page.getByLabel(/^Done window/).fill('99');
    await page.getByRole('button', { name: 'Save filter' }).click();
    await expect(page.getByRole('alert').filter({ hasText: 'validation: done_days must be 0-90' })).toBeVisible();
    await expect(page.getByLabel(/^Done window/)).toHaveValue('99');
  });

  test('Linear tab uses "Teams"; both tabs switch', async ({ page, h }) => {
    const linear = { ...jiraView('x'), source: 'linear', site: null, tickets: [], filter: { statuses: ['Todo'], done_days: 0, projects: ['ENG'], auto_queue: [] } };
    await mockSources(page, { summaries: summaries(true, true), views: { jira: jiraView('t-open'), linear } });
    await open(page, h);
    await expect(page.getByRole('tab')).toHaveText(['Linear', 'Jira']);
    await expect(page.getByLabel(/^Teams/)).toHaveValue('ENG');
    await expect(page.locator('[data-column="source-done"]')).toHaveCount(0);
    await page.getByRole('tab', { name: 'Jira' }).click();
    await expect(page.getByLabel(/^Projects/)).toBeVisible();
  });
});
