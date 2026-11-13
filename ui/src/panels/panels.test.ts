import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { HOSTILE, NOW, fakeTransport, message, overview, session, task } from '../fixtures';
import { setTransport } from '../lib/api';
import Activity from './Activity.svelte';
import Connect from './Connect.svelte';
import Decisions from './Decisions.svelte';
import Header from './Header.svelte';
import Messages from './Messages.svelte';
import Sessions from './Sessions.svelte';
import Tasks from './Tasks.svelte';

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
