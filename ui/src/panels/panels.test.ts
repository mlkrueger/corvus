import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { HOSTILE, NOW, agent, conversation, fakeTransport, message, overview, session, task } from '../fixtures';
import { setTransport } from '../lib/api';
import Activity from './Activity.svelte';
import Connect from './Connect.svelte';
import Decisions from './Decisions.svelte';
import Header from './Header.svelte';
import Messages from './Messages.svelte';
import Sessions from './Sessions.svelte';
import Tasks from './Tasks.svelte';
import UpdateBanner from './UpdateBanner.svelte';
import { UpdateState } from '../lib/update.svelte';

let restore: () => void = () => {};
afterEach(() => restore());

describe('Header', () => {
  const props = (over = {}) => ({
    overview: overview(),
    paused: false,
    dark: false,
    error: null,
    onTogglePause: vi.fn(),
    onToggleTheme: vi.fn(),
    onUnreadClick: vi.fn(),
    onConnect: vi.fn(),
    ...over,
  });

  it('shows presence, task, unread, expired-lease and blocked-by-cancelled chips', () => {
    render(Header, props());
    const chip = (name: string) => document.querySelector(`[data-chip="${name}"]`)!;
    expect(chip('presence-working')).toHaveTextContent('working 1');
    expect(chip('open')).toHaveTextContent('open tasks 2');
    expect(chip('claimed')).toHaveTextContent('claimed 1');
    expect(chip('unread')).toHaveTextContent('unread 3');
    expect(chip('expired')).toHaveTextContent('expired leases 1');
    expect(chip('blocked-cancelled')).toHaveTextContent('blocked by cancelled 1');
  });

  it('the unread chip, pause and theme buttons call back', async () => {
    const p = props();
    render(Header, p);
    await fireEvent.click(screen.getByRole('button', { name: /unread/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Pause' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Toggle theme' }));
    expect(p.onUnreadClick).toHaveBeenCalledOnce();
    expect(p.onTogglePause).toHaveBeenCalledOnce();
    expect(p.onToggleTheme).toHaveBeenCalledOnce();
  });

  it('labels the button Resume when paused and renders before data arrives', () => {
    render(Header, props({ overview: null, paused: true }));
    expect(screen.getByRole('button', { name: 'Resume' })).toBeInTheDocument();
    expect(screen.getByText('loading...')).toBeInTheDocument();
  });
});

describe('Sessions', () => {
  const o = overview();
  const tasks = [...o.tasks.claimed, ...o.tasks.open];

  it('shows live sessions with client, unread, current task and lease countdown; hides offline/ended', () => {
    render(Sessions, { sessions: o.sessions, tasks, now: NOW });
    const alice = document.querySelector('[data-session="alice"]') as HTMLElement;
    expect(alice).toHaveTextContent('working');
    expect(alice).toHaveTextContent('claude');
    expect(alice).toHaveTextContent('2 unread');
    expect(alice).toHaveTextContent('Index the repo');
    expect(alice).toHaveTextContent('10m 00s left');
    expect(alice).toHaveTextContent('last seen 30s ago');
    expect(document.querySelector('[data-session="bob"]')).toBeInTheDocument();
    expect(document.querySelector('[data-session="carol"]')).toBeNull();
    expect(document.querySelector('[data-session="dave"]')).toBeNull();
  });

  it('the toggle reveals offline and ended sessions', async () => {
    render(Sessions, { sessions: o.sessions, tasks, now: NOW });
    await fireEvent.click(screen.getByLabelText(/show offline/));
    expect(document.querySelector('[data-session="carol"]')).toBeInTheDocument();
    expect(document.querySelector('[data-session="dave"]')).toHaveTextContent('ended 5m 00s ago (ended)');
  });

  it('flags an expired lease', () => {
    const s = session({ current_task: 'k-1', lease_expires_at: new Date(NOW - 90_000).toISOString() });
    render(Sessions, { sessions: [s], tasks, now: NOW });
    expect(screen.getByText('expired 1m 30s ago')).toHaveClass('warn');
  });

  it('renders hostile metadata and names as literal text', () => {
    const s = session({ agent_name: HOSTILE, metadata: { note: HOSTILE } });
    const { container } = render(Sessions, { sessions: [s], tasks: [], now: NOW });
    expect(container.querySelector('img')).toBeNull();
    expect(container).toHaveTextContent(HOSTILE);
  });

  it('empty state', () => {
    render(Sessions, { sessions: [], tasks: [], now: NOW });
    expect(screen.getByText('No live sessions.')).toBeInTheDocument();
  });

  it('shows the tool and directory of the agent\'s conversation', () => {
    render(Sessions, { sessions: o.sessions, agents: o.agents, tasks, now: NOW });
    expect(document.querySelector('[data-session="alice"] .conversation')).toHaveTextContent('claude-code · /work/project');
    expect(document.querySelector('[data-session="bob"] .conversation')).toBeNull();
    // Reachable agents, and unreachable ones with no mail, are not flagged.
    expect(document.querySelector('[data-stranded]')).toBeNull();
  });

  it('flags an unreachable agent with unread mail and reassigns it', async () => {
    const t = fakeTransport({ 'POST /api/admin/reassign-mail': { from: 'dave', to: 'bob', moved: [] } });
    restore = setTransport(t.transport);
    const onChanged = vi.fn();
    const agents = [
      agent({ name: 'bob' }),
      agent({ name: 'dave', reachable: false, unread_direct: 3, conversations: [conversation({ agent_name: 'dave', tool: 'opencode', directory: '/work/old' })] }),
      agent({ name: 'erin', reachable: false }),
    ];
    render(Sessions, { sessions: [], agents, tasks: [], now: NOW, onChanged });
    const row = document.querySelector('[data-stranded="dave"]') as HTMLElement;
    expect(row).toHaveTextContent('not reachable');
    expect(row).toHaveTextContent('3 unread');
    expect(row).toHaveTextContent('opencode · /work/old');
    expect(document.querySelectorAll('[data-stranded]')).toHaveLength(1);
    const button = screen.getByRole('button', { name: 'Reassign' });
    expect(button).toBeDisabled();
    const select = screen.getByLabelText('Reassign dave to') as HTMLSelectElement;
    expect([...select.options].map((x) => x.value)).toEqual(['', 'bob', 'erin']);
    await fireEvent.change(select, { target: { value: 'bob' } });
    await fireEvent.click(button);
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(t.api()).toHaveLength(1);
    expect(t.api()[0]).toMatchObject({ method: 'POST', path: '/api/admin/reassign-mail', body: { from: 'dave', to: 'bob' } });
  });

  it('shows the server error when reassigning fails', async () => {
    const t = fakeTransport({
      'POST /api/admin/reassign-mail': () => ({ status: 404, body: { error: { code: 'not_found', message: 'agent bob not found' } } }),
    });
    restore = setTransport(t.transport);
    const onChanged = vi.fn();
    const agents = [agent({ name: 'bob' }), agent({ name: HOSTILE, reachable: false, unread_direct: 1 })];
    const { container } = render(Sessions, { sessions: [], agents, tasks: [], now: NOW, onChanged });
    expect(container.querySelector('img')).toBeNull();
    await fireEvent.change(container.querySelector('select') as HTMLSelectElement, { target: { value: 'bob' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Reassign' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('agent bob not found');
    expect(onChanged).not.toHaveBeenCalled();
  });
});

describe('Activity and Decisions', () => {
  it('Activity lists events', () => {
    render(Activity, { items: overview().events, now: NOW });
    expect(screen.getByText('task.claimed')).toBeInTheDocument();
    expect(screen.getByText('alice claimed Index the repo')).toBeInTheDocument();
  });
  it('Activity and Decisions render untrusted text literally', () => {
    const o = overview();
    const ev = { ...o.events[0]!, summary: HOSTILE };
    const dec = { ...o.decisions[0]!, title: HOSTILE, decision: '<script>window.__pwned=1</script>' };
    const a = render(Activity, { items: [ev], now: NOW });
    const d = render(Decisions, { decisions: [dec], now: NOW });
    for (const c of [a.container, d.container]) {
      expect(c.querySelector('img')).toBeNull();
      expect(c.querySelector('script')).toBeNull();
    }
    expect(a.container).toHaveTextContent(HOSTILE);
    expect(d.container).toHaveTextContent('<script>window.__pwned=1</script>');
  });
  it('Decisions lists title, decision, rationale and author', () => {
    render(Decisions, { decisions: overview().decisions, now: NOW });
    expect(screen.getByText('Use SQLite')).toBeInTheDocument();
    expect(screen.getByText('We use SQLite.')).toBeInTheDocument();
    expect(screen.getByText('Local-first.')).toBeInTheDocument();
    expect(screen.getByText(/operator · 15m 00s ago/)).toBeInTheDocument();
  });
});

describe('Messages', () => {
  const msgs = [
    message({ id: 1, sender: 'alice', recipient: 'operator', type: 'question', body: 'Which db?', thread_title: 'DB choice', read_by: ['operator'] }),
    message({ id: 2, sender: 'operator', recipient: null, body: 'Stand up', thread_title: 'Operator broadcasts', unread_by: ['alice', 'bob'] }),
  ];
  const base = () => ({
    messages: msgs,
    filter: { mode: 'all' } as const,
    agents: ['alice', 'bob'],
    operator: 'operator',
    now: NOW,
    onFilter: vi.fn(),
    onChanged: vi.fn(),
  });

  it('lists newest first with thread title, recipient and read/unread-by', () => {
    render(Messages, base());
    const items = document.querySelectorAll('[data-message]');
    expect([...items].map((i) => i.getAttribute('data-message'))).toEqual(['2', '1']);
    expect(items[0]).toHaveTextContent('to everyone');
    expect(items[0]).toHaveTextContent('thread: Operator broadcasts');
    expect(items[0]).toHaveTextContent('unread by alice, bob');
    expect(items[1]).toHaveTextContent('read by operator');
    expect(items[1]).toHaveTextContent('thread: DB choice');
  });

  it('applies the unread filter and the agent filter', () => {
    const { unmount } = render(Messages, { ...base(), filter: { mode: 'unread' } });
    expect(document.querySelectorAll('[data-message]')).toHaveLength(1);
    unmount();
    render(Messages, { ...base(), filter: { mode: 'agent', agent: 'alice' } });
    expect(document.querySelectorAll('[data-message]')).toHaveLength(1);
    expect(document.querySelector('[data-message="1"]')).toBeInTheDocument();
  });

  it('choosing a filter calls onFilter', async () => {
    const p = base();
    render(Messages, p);
    const sel = screen.getByLabelText('Message filter');
    await fireEvent.change(sel, { target: { value: 'unread' } });
    await fireEvent.change(sel, { target: { value: 'agent:bob' } });
    await fireEvent.change(sel, { target: { value: 'all' } });
    expect(p.onFilter.mock.calls.map((c) => c[0])).toEqual([{ mode: 'unread' }, { mode: 'agent', agent: 'bob' }, { mode: 'all' }]);
  });

  it('compose to one agent posts a direct message as the operator', async () => {
    const t = fakeTransport({ 'POST /api/messages/direct': { id: 3 } });
    restore = setTransport(t.transport);
    const p = base();
    render(Messages, p);
    await fireEvent.change(screen.getByLabelText('Recipient'), { target: { value: 'bob' } });
    await fireEvent.input(screen.getByLabelText('Message body'), { target: { value: 'ping' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(p.onChanged).toHaveBeenCalled());
    expect(t.api()).toHaveLength(1);
    expect(t.api()[0]).toMatchObject({ path: '/api/messages/direct', body: { from: 'operator', to: 'bob', body: 'ping', type: 'note' } });
    expect(screen.getByLabelText('Message body')).toHaveValue('');
  });

  it('compose to everyone creates the keyed thread then posts without `to`', async () => {
    const t = fakeTransport({ 'POST /api/threads': { id: 'tb' }, 'POST /api/messages': { id: 3 } });
    restore = setTransport(t.transport);
    render(Messages, base());
    await fireEvent.input(screen.getByLabelText('Message body'), { target: { value: 'all' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(t.api()).toHaveLength(2));
    expect(t.api()[0]!.path).toBe('/api/threads');
    expect(t.api()[1]).toMatchObject({ path: '/api/messages', body: { thread_id: 'tb', to: null, body: 'all' } });
  });

  it('"wake everyone" is offered only for a broadcast and is sent as `wake`', async () => {
    const t = fakeTransport({ 'POST /api/threads': { id: 'tb' }, 'POST /api/messages': { id: 3 } });
    restore = setTransport(t.transport);
    render(Messages, base());
    const box = screen.getByLabelText('Wake everyone') as HTMLInputElement;
    expect(box.checked).toBe(false);
    await fireEvent.click(box);
    await fireEvent.input(screen.getByLabelText('Message body'), { target: { value: 'stop now' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(t.api()).toHaveLength(2));
    expect(t.api()[1]).toMatchObject({ path: '/api/messages', body: { to: null, wake: true } });
    // One use: the next broadcast does not wake unless asked again.
    await waitFor(() => expect((screen.getByLabelText('Wake everyone') as HTMLInputElement).checked).toBe(false));
    await fireEvent.input(screen.getByLabelText('Message body'), { target: { value: 'fyi' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(t.api()).toHaveLength(4));
    expect(t.api()[3]).toMatchObject({ path: '/api/messages', body: { to: null, wake: false } });
    // Not offered for a direct message.
    await fireEvent.change(screen.getByLabelText('Recipient'), { target: { value: 'bob' } });
    expect(screen.queryByLabelText('Wake everyone')).toBeNull();
  });

  it('shows a reassignment as a move, never as read', () => {
    const moved = [
      message({ id: 5, sender: 'operator', recipient: 'old', reassigned_to: 'new' }),
      message({ id: 6, sender: 'operator', recipient: 'new', reassigned_from: 5, unread_by: ['new'] }),
    ];
    render(Messages, { ...base(), messages: moved });
    const original = document.querySelector('[data-message="5"]') as HTMLElement;
    expect(original).toHaveTextContent('reassigned to new');
    expect(original).not.toHaveTextContent('read by');
    expect(document.querySelector('[data-message="6"]')).toHaveTextContent('moved here from message 5');
  });

  it('reply to a question defaults to answer; reply to a note defaults to note', async () => {
    const t = fakeTransport({ 'POST /api/messages': { id: 9 } });
    restore = setTransport(t.transport);
    render(Messages, base());
    const q = document.querySelector('[data-message="1"]') as HTMLElement;
    await fireEvent.click(within(q).getByRole('button', { name: 'Reply' }));
    expect(within(q).getByLabelText('Reply type')).toHaveValue('answer');
    await fireEvent.input(within(q).getByLabelText('Reply body'), { target: { value: 'sqlite' } });
    await fireEvent.click(within(q).getByRole('button', { name: 'Send reply' }));
    await waitFor(() => expect(t.api()).toHaveLength(1));
    expect(t.api()[0]).toMatchObject({ body: { in_reply_to: 1, type: 'answer', thread_id: 't-1', to: 'alice', body: 'sqlite' } });

    const n = document.querySelector('[data-message="2"]') as HTMLElement;
    await fireEvent.click(within(n).getByRole('button', { name: 'Reply' }));
    expect(within(n).getByLabelText('Reply type')).toHaveValue('note');
  });

  it('shows the server error when sending fails and keeps the text', async () => {
    const t = fakeTransport({
      'POST /api/messages/direct': () => ({ status: 400, body: { error: { code: 'validation', message: 'body too long' } } }),
    });
    restore = setTransport(t.transport);
    const p = base();
    render(Messages, p);
    await fireEvent.change(screen.getByLabelText('Recipient'), { target: { value: 'bob' } });
    await fireEvent.input(screen.getByLabelText('Message body'), { target: { value: 'x' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('validation: body too long');
    expect(screen.getByLabelText('Message body')).toHaveValue('x');
    expect(p.onChanged).not.toHaveBeenCalled();
  });

  it('does not send an empty message', () => {
    render(Messages, base());
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
  });

  it('renders hostile bodies, titles and names as literal text', () => {
    const hostile = message({ id: 5, body: HOSTILE, thread_title: '<script>window.__pwned=1</script>', sender: HOSTILE });
    const { container } = render(Messages, { ...base(), messages: [hostile] });
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('script')).toBeNull();
    expect(container).toHaveTextContent(HOSTILE);
    expect(container).toHaveTextContent('<script>window.__pwned=1</script>');
  });
});

describe('Tasks', () => {
  const base = () => ({ tasks: overview().tasks, operator: 'operator', now: NOW, onChanged: vi.fn() });

  it('renders the four columns with priority, blocked flags and holder', () => {
    render(Tasks, base());
    const col = (k: string) => document.querySelector(`[data-column="${k}"]`) as HTMLElement;
    expect(col('open')).toHaveTextContent('Write the parser');
    expect(col('open')).toHaveTextContent('P1');
    expect(col('open').querySelector('[data-flag="blocked"]')).toBeInTheDocument();
    expect(col('open').querySelector('[data-flag="blocked-by-cancelled"]')).toBeInTheDocument();
    expect(col('claimed')).toHaveTextContent('held by alice');
    expect(col('claimed')).toHaveTextContent('P5');
    expect(col('done')).toHaveTextContent('Ship it');
    expect(col('cancelled')).toHaveTextContent('reason: no longer needed');
  });

  it('only open and claimed tasks can be cancelled', () => {
    render(Tasks, base());
    const col = (k: string) => document.querySelector(`[data-column="${k}"]`) as HTMLElement;
    expect(within(col('open')).getAllByRole('button', { name: 'Cancel' })).toHaveLength(2);
    expect(within(col('claimed')).getAllByRole('button', { name: 'Cancel' })).toHaveLength(1);
    expect(within(col('done')).queryByRole('button')).toBeNull();
    expect(within(col('cancelled')).queryByRole('button')).toBeNull();
  });

  it('cancel posts by the operator with the reason', async () => {
    const t = fakeTransport({ 'POST /api/tasks/': { id: 'o-1' } });
    restore = setTransport(t.transport);
    const p = base();
    render(Tasks, p);
    const card = document.querySelector('[data-task="o-1"]') as HTMLElement;
    await fireEvent.click(within(card).getByRole('button', { name: 'Cancel' }));
    await fireEvent.input(within(card).getByLabelText('Cancel reason'), { target: { value: 'dup' } });
    await fireEvent.click(within(card).getByRole('button', { name: 'Confirm cancel' }));
    await waitFor(() => expect(p.onChanged).toHaveBeenCalled());
    expect(t.api()[0]).toMatchObject({ method: 'POST', path: '/api/tasks/o-1/cancel', body: { by: 'operator', reason: 'dup', cascade: false } });
  });

  it('renders hostile titles as literal text', () => {
    const o = overview();
    o.tasks.open = [task({ id: 'h-1', title: HOSTILE })];
    const { container } = render(Tasks, { ...base(), tasks: o.tasks });
    expect(container.querySelector('img')).toBeNull();
    expect(container).toHaveTextContent(HOSTILE);
  });
});

describe('Connect', () => {
  const info = {
    version: '0.1.0',
    corvus_path: '/Applications/Corvus.app/corvus',
    cli_installed: false,
    adapters: [{ name: 'claude-code', installed: true }],
    mcp_snippet: '{"mcpServers":{}}',
    hook_snippet: 'hook-config',
  };

  it('shows app_info: path, adapters and snippets', async () => {
    restore = setTransport(fakeTransport({ app_info: info }).transport);
    render(Connect, { onClose: vi.fn() });
    expect(await screen.findByText('/Applications/Corvus.app/corvus')).toBeInTheDocument();
    expect(screen.getByText('claude-code')).toBeInTheDocument();
    expect(screen.getByText('{"mcpServers":{}}')).toBeInTheDocument();
    expect(screen.getByText('hook-config')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Install command-line tool' })).toBeInTheDocument();
  });

  it('install_cli then reloads app_info', async () => {
    let installed = false;
    const t = fakeTransport({
      app_info: () => ({ ...info, cli_installed: installed }),
      install_cli: () => {
        installed = true;
        return null;
      },
    });
    restore = setTransport(t.transport);
    render(Connect, { onClose: vi.fn() });
    await fireEvent.click(await screen.findByRole('button', { name: 'Install command-line tool' }));
    expect(await screen.findByText(/installed on PATH/)).toBeInTheDocument();
    expect(t.calls.map((c) => c.cmd)).toEqual(['app_info', 'install_cli', 'app_info']);
  });

  it('falls back gracefully when app_info fails', async () => {
    restore = setTransport(fakeTransport({}).transport);
    render(Connect, { onClose: vi.fn() });
    expect(await screen.findByRole('status')).toHaveTextContent('only available in the desktop app');
  });
});

describe('Updates', () => {
  const status = (over = {}) => ({ enabled: true, current: '0.1.0', available: null, error: null, ...over });
  const info = {
    version: '0.1.0',
    corvus_path: '/Applications/Corvus.app/corvus',
    cli_installed: true,
    adapters: [],
    mcp_snippet: '{}',
    hook_snippet: 'hook',
  };

  it('the banner is absent with no update and offers the install when there is one', async () => {
    const t = fakeTransport({ check_update: status(), install_update: null });
    restore = setTransport(t.transport);
    const updates = new UpdateState();
    render(UpdateBanner, { updates });
    await updates.check();
    expect(screen.queryByRole('status', { name: 'Update available' })).toBeNull();

    updates.status = status({ available: '0.2.0' });
    const banner = await screen.findByRole('status', { name: 'Update available' });
    expect(banner).toHaveTextContent('Corvus 0.2.0 is available (this is 0.1.0).');
    await fireEvent.click(screen.getByRole('button', { name: 'Install and restart' }));
    expect(t.calls.map((c) => c.cmd)).toEqual(['check_update', 'install_update']);
    expect(t.calls[0].args).toEqual({ force: false });
  });

  it('a failed install is shown and the button comes back', async () => {
    const t = fakeTransport({
      install_update: () => {
        throw new Error('cannot download the update: offline');
      },
    });
    restore = setTransport(t.transport);
    const updates = new UpdateState();
    updates.status = status({ available: '0.2.0' });
    render(UpdateBanner, { updates });
    await fireEvent.click(screen.getByRole('button', { name: 'Install and restart' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('cannot download the update: offline');
    expect(screen.getByRole('button', { name: 'Install and restart' })).toBeEnabled();
  });

  it('a background check that fails stays silent; a requested one reports it', async () => {
    restore = setTransport(fakeTransport({ check_update: status({ error: 'cannot check for updates: offline' }) }).transport);
    const updates = new UpdateState();
    await updates.check();
    expect(updates.error).toBeNull();
    await updates.check(true);
    expect(updates.error).toBe('cannot check for updates: offline');
  });

  it('outside the app a background check does nothing', async () => {
    restore = setTransport(fakeTransport({}).transport);
    const updates = new UpdateState();
    await updates.check();
    expect(updates.status).toBeNull();
    expect(updates.error).toBeNull();
  });

  it('Connect: "Check for updates" forces a check and reports the result', async () => {
    const t = fakeTransport({ app_info: info, check_update: status({ available: '0.2.0' }) });
    restore = setTransport(t.transport);
    const updates = new UpdateState();
    render(Connect, { onClose: vi.fn(), updates });
    await fireEvent.click(await screen.findByRole('button', { name: 'Check for updates' }));
    expect(await screen.findByText(/Version 0.2.0 is available/)).toBeInTheDocument();
    expect(t.calls.find((c) => c.cmd === 'check_update')?.args).toEqual({ force: true });
  });

  it('Connect: says so when checks are turned off, with no button', async () => {
    restore = setTransport(fakeTransport({ app_info: info }).transport);
    const updates = new UpdateState();
    updates.status = status({ enabled: false });
    render(Connect, { onClose: vi.fn(), updates });
    expect(await screen.findByTestId('update-line')).toHaveTextContent('Update checks are turned off');
    expect(screen.queryByRole('button', { name: 'Check for updates' })).toBeNull();
  });

  it('Connect: up to date', async () => {
    restore = setTransport(fakeTransport({ app_info: info }).transport);
    const updates = new UpdateState();
    updates.status = status();
    render(Connect, { onClose: vi.fn(), updates });
    expect(await screen.findByTestId('update-line')).toHaveTextContent('Up to date.');
  });
});
