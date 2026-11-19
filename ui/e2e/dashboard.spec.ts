import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { findInDir } from '../scripts/scan-no-data.mjs';
import { DIST, IPC_SHIM, UI_DIR, type Harness } from './harness';
import { HOSTILE_IMG, HOSTILE_SCRIPT, expect, open, test, waitForPolls } from './fixtures';

const SHOTS = join(UI_DIR, 'e2e', 'screenshots');

interface MessageRow {
  id: number;
  thread_id: string;
  sender: string;
  recipient: string | null;
  type: string;
  body: string;
  in_reply_to: number | null;
  thread_title: string;
  read_by: string[];
  unread_by: string[];
}

const messages = (h: Harness) => h.api<MessageRow[]>('GET', '/api/messages?limit=100');

function seedBasic(h: Harness) {
  const alice = h.cliJson<{ id: string }>('session', 'register', '--agent', 'alice', '--client', 'claude');
  h.cliJson('session', 'register', '--agent', 'bob', '--client', 'codex');
  const a = h.cliJson<{ id: string }>('task', 'create', '--title', 'Write the parser', '--priority', '2');
  h.cliJson('task', 'create', '--title', 'Index the repo', '--priority', '5');
  h.cliJson('task', 'claim', '--session', alice.id);
  h.cli('tell', 'alice', 'Please start on the parser');
  h.cli('tell', 'bob', 'Hello bob', '--type', 'status');
  h.cli('decision', 'record', '--title', 'Use SQLite', '--rationale', 'Local first', 'We use SQLite');
  return { alice, a };
}

test.describe('dashboard in a browser engine', () => {
  test('renders seeded data in every panel', async ({ page, h, probe }) => {
    seedBasic(h);
    await open(page, h);
    await expect(page.locator('[data-session="alice"]')).toContainText('claude');
    await expect(page.locator('[data-session="alice"]')).toContainText('left');
    await expect(page.locator('[data-session="bob"]')).toBeVisible();
    await expect(page.locator('[data-chip="unread"]')).toContainText('2');
    await expect(page.locator('[data-chip="open"]')).toContainText('1');
    await expect(page.locator('[data-chip="claimed"]')).toContainText('1');
    await expect(page.getByText('Please start on the parser')).toBeVisible();
    await expect(page.getByText('Use SQLite', { exact: true })).toBeVisible();
    await expect(page.locator('[data-column="open"]')).toContainText('Write the parser');
    await expect(page.locator('[data-column="claimed"]')).toContainText('Index the repo');
    await expect(page.locator('[data-column="claimed"]')).toContainText('held by alice');
    expect(await probe.violations()).toEqual([]);
    expect(probe.consoleErrors).toEqual([]);
  });

  test('hostile text renders literally: no script runs, no CSP violation', async ({ page, h, probe }) => {
    h.cliJson('session', 'register', '--agent', 'mallory', '--client', 'claude', '--metadata', JSON.stringify({ note: HOSTILE_IMG, s: HOSTILE_SCRIPT }));
    h.cliJson('task', 'create', '--title', HOSTILE_IMG, '--body', HOSTILE_SCRIPT);
    h.cli('tell', 'mallory', HOSTILE_IMG);
    h.cliJson('session', 'register', '--agent', 'trudy', '--client', 'claude');
    h.cli('tell', 'mallory', HOSTILE_SCRIPT, '--from', 'trudy');
    h.cli('decision', 'record', '--title', HOSTILE_IMG, '--rationale', HOSTILE_SCRIPT, HOSTILE_SCRIPT + HOSTILE_IMG);
    await open(page, h);

    // Each hostile string shows up as text in every place the data is shown.
    await expect(page.locator('[data-session="mallory"] .meta')).toContainText(HOSTILE_IMG);
    await expect(page.locator('[data-column="open"] .card-title')).toHaveText(HOSTILE_IMG);
    await expect(page.locator('.message .body', { hasText: HOSTILE_SCRIPT })).toHaveCount(1);
    await expect(page.locator('.message .body', { hasText: HOSTILE_IMG })).toHaveCount(1);
    await expect(page.locator('.decision strong')).toHaveText(HOSTILE_IMG);
    await expect(page.locator('.decision .body').first()).toContainText(HOSTILE_SCRIPT + HOSTILE_IMG);

    // Give any (wrongly created) <img onerror> time to fire.
    await page.waitForTimeout(1000);
    expect(await page.evaluate(() => (window as unknown as { __pwned?: unknown }).__pwned)).toBeUndefined();
    expect(await page.locator('#app img').count()).toBe(0);
    expect(await page.locator('#app script').count()).toBe(0);
    expect(await probe.violations()).toEqual([]);
    expect(probe.consoleErrors).toEqual([]);
  });

  test('compose to one agent sends a direct message as the operator', async ({ page, h }) => {
    seedBasic(h);
    await open(page, h);
    await page.getByLabel('Recipient').selectOption('bob');
    await page.getByLabel('Message body').fill('direct from the UI');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(page.locator('.message', { hasText: 'direct from the UI' })).toBeVisible();
    const m = (await messages(h)).find((x) => x.body === 'direct from the UI')!;
    expect(m).toMatchObject({ sender: 'operator', recipient: 'bob', type: 'note' });
    await expect(page.getByLabel('Message body')).toHaveValue('');
  });

  test('compose to everyone broadcasts through the operator thread (reused on the second send)', async ({ page, h }) => {
    seedBasic(h);
    await open(page, h);
    for (const text of ['first broadcast', 'second broadcast']) {
      await page.getByLabel('Recipient').selectOption('');
      await page.getByLabel('Message type').selectOption('status');
      await page.getByLabel('Message body').fill(text);
      await page.getByRole('button', { name: 'Send', exact: true }).click();
      await expect(page.locator('.message', { hasText: text })).toBeVisible();
    }
    const all = await messages(h);
    const a = all.find((x) => x.body === 'first broadcast')!;
    const b = all.find((x) => x.body === 'second broadcast')!;
    expect(a).toMatchObject({ sender: 'operator', recipient: null, type: 'status', thread_title: 'Operator broadcasts' });
    expect(b).toMatchObject({ sender: 'operator', recipient: null, thread_title: 'Operator broadcasts' });
    expect(a.thread_id).toBe(b.thread_id);
  });

  test('reply to a question defaults to answer, in the thread, in_reply_to the question', async ({ page, h }) => {
    h.cliJson('session', 'register', '--agent', 'alice', '--client', 'claude');
    const thread = await h.api<{ id: string }>('POST', '/api/threads', { creator: 'alice', title: 'DB choice', task_id: null, key: null, session_id: null });
    h.cli('msg', 'post', '--thread', thread.id, '--from', 'alice', '--type', 'question', 'Which database should we use?');
    h.cli('msg', 'post', '--thread', thread.id, '--from', 'alice', '--type', 'note', 'FYI the build is green');
    await open(page, h);
    const q = page.locator('.message', { hasText: 'Which database should we use?' });
    await q.getByRole('button', { name: 'Reply' }).click();
    await expect(q.getByLabel('Reply type')).toHaveValue('answer');
    await q.getByLabel('Reply body').fill('SQLite, please');
    await q.getByRole('button', { name: 'Send reply' }).click();
    await expect(page.locator('.message', { hasText: 'SQLite, please' })).toBeVisible();

    const all = await messages(h);
    const question = all.find((x) => x.type === 'question')!;
    const reply = all.find((x) => x.body === 'SQLite, please')!;
    expect(reply).toMatchObject({ type: 'answer', sender: 'operator', recipient: 'alice', in_reply_to: question.id });

    const n = page.locator('.message', { hasText: 'FYI the build is green' });
    await n.getByRole('button', { name: 'Reply' }).click();
    await expect(n.getByLabel('Reply type')).toHaveValue('note');
  });

  test('viewing creates no receipts: no writes, stats and read-by unchanged', async ({ page, h }) => {
    seedBasic(h);
    const before = { stats: h.cli('stats'), msgs: await messages(h) };
    expect(before.msgs.every((m) => m.read_by.length === 0)).toBe(true);
    await open(page, h);
    await page.getByRole('button', { name: /unread/ }).click();
    await expect(page.getByLabel('Message filter')).toHaveValue('unread');
    await waitForPolls(h, 2, 12_000);
    await expect(page.getByText('Please start on the parser')).toBeVisible();
    expect(h.requests.length).toBeGreaterThan(2);
    expect(h.requests.every((r) => r.method === 'GET')).toBe(true);
    expect(h.requests.some((r) => /\/ack\b/.test(r.path))).toBe(false);
    expect(h.cli('stats')).toBe(before.stats);
    expect(await messages(h)).toEqual(before.msgs);
  });

  test('cancel an open task from the board; its dependent shows blocked-by-cancelled', async ({ page, h }) => {
    const a = h.cliJson<{ id: string }>('task', 'create', '--title', 'Prerequisite work');
    h.cliJson('task', 'create', '--title', 'Dependent work', '--depends-on', a.id);
    await open(page, h);
    const dependent = page.locator('[data-task]', { hasText: 'Dependent work' });
    await expect(dependent.locator('[data-flag="blocked"]')).toBeVisible();
    await expect(dependent.locator('[data-flag="blocked-by-cancelled"]')).toHaveCount(0);

    const card = page.locator('[data-task]', { hasText: 'Prerequisite work' });
    await card.getByRole('button', { name: 'Cancel' }).click();
    await card.getByLabel('Cancel reason').fill('superseded');
    await card.getByRole('button', { name: 'Confirm cancel' }).click();

    await expect(page.locator('[data-column="cancelled"]')).toContainText('Prerequisite work');
    await expect(page.locator('[data-column="cancelled"]')).toContainText('reason: superseded');
    await expect(dependent.locator('[data-flag="blocked-by-cancelled"]')).toBeVisible();
    await expect(page.locator('[data-chip="blocked-cancelled"]')).toContainText('1');
    const t = await h.api<{ status: string; cancelled_by: string }>('GET', `/api/tasks/${a.id}`);
    expect(t).toMatchObject({ status: 'cancelled', cancelled_by: 'operator' });
  });

  test('pause stops requests; resume restarts them', async ({ page, h }) => {
    seedBasic(h);
    await open(page, h);
    await waitForPolls(h, 2, 12_000);
    await page.getByRole('button', { name: 'Pause' }).click();
    await expect(page.getByRole('button', { name: 'Resume' })).toBeVisible();
    await page.waitForTimeout(500); // let an in-flight poll finish
    h.resetRequests();
    await page.waitForTimeout(7000); // > two poll intervals
    expect(h.requests).toEqual([]);
    await page.getByRole('button', { name: 'Resume' }).click();
    await waitForPolls(h, 1, 5000);
  });

  test('Connect an agent view shows app_info from the shim', async ({ page, h }) => {
    await open(page, h);
    await page.getByRole('button', { name: 'Connect an agent' }).click();
    await expect(page.getByText('/opt/corvus/bin/corvus', { exact: true })).toBeVisible();
    await expect(page.getByText('claude-code')).toBeVisible();
    await expect(page.locator('.snippet').first()).toContainText('"args":["mcp"]');
    const atLogin = page.getByRole('checkbox', { name: 'Start Corvus at login' });
    await expect(atLogin).toBeChecked();
    await atLogin.uncheck();
    await expect(atLogin).not.toBeChecked();
  });

  test('narrow viewport: one column, no page overflow; screenshot', async ({ page, h }, info) => {
    seedBasic(h);
    await page.setViewportSize({ width: 600, height: 1000 });
    await open(page, h);
    await expect(page.getByText('Please start on the parser')).toBeVisible();
    const { sw, cw } = await page.evaluate(() => ({ sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth }));
    expect(sw).toBeLessThanOrEqual(cw);
    const xs = await page.locator('.grid > div').evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().left)));
    expect(new Set(xs).size).toBe(1); // single column
    const boardScrolls = await page.locator('.board').evaluate((e) => e.scrollWidth > e.clientWidth);
    expect(boardScrolls).toBe(true);
    mkdirSync(SHOTS, { recursive: true });
    await page.screenshot({ path: join(SHOTS, `narrow-600-${info.project.name}.png`), fullPage: true });
  });

  test('dark theme: follows the system, toggle overrides and persists; screenshot', async ({ browser, h }, info) => {
    seedBasic(h);
    const ctx = await browser.newContext({ colorScheme: 'dark', viewport: { width: 1280, height: 900 } });
    const page = await ctx.newPage();
    await page.addInitScript(IPC_SHIM);
    await open(page, h);
    await expect(page.getByText('Please start on the parser')).toBeVisible();
    const bg = () => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    const darkBg = await bg();
    mkdirSync(SHOTS, { recursive: true });
    await page.screenshot({ path: join(SHOTS, `dark-${info.project.name}.png`), fullPage: true });

    await page.getByRole('button', { name: 'Toggle theme' }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    expect(await bg()).not.toBe(darkBg);
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await page.getByRole('button', { name: 'Toggle theme' }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    expect(await bg()).toBe(darkBg);
    await page.screenshot({ path: join(SHOTS, `dark-toggled-${info.project.name}.png`), fullPage: true });
    await ctx.close();
  });

  test('bundle has no data: no seeded string appears under ui/dist', async ({ h }) => {
    const needles = ['zz-distinct-agent-7f3a', 'zz-distinct-task-91bc', 'zz-distinct-message-2d40', 'zz-distinct-decision-c8e1'];
    h.cliJson('session', 'register', '--agent', needles[0]!, '--client', 'claude');
    h.cliJson('task', 'create', '--title', needles[1]!);
    h.cli('tell', needles[0]!, needles[2]!);
    h.cli('decision', 'record', '--title', needles[3]!, needles[3]!);
    // The seeded data really is in the database...
    expect(JSON.stringify(await h.api('GET', '/api/overview'))).toContain(needles[1]);
    // ...and nowhere in the built assets.
    expect(findInDir(DIST, [...needles, h.home])).toEqual([]);
  });
});
