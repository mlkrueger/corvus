import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import App from './App.svelte';
import { fakeTransport, message, overview, sourceView, ticket } from './fixtures';
import { setTransport } from './lib/api';

let restore: () => void = () => {};
afterEach(() => {
  restore();
  localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
});
beforeEach(() => localStorage.clear());

function boot() {
  const t = fakeTransport({
    'GET /api/overview': overview(),
    'GET /api/messages': [message({ id: 1, body: 'hello there', unread_by: ['operator'] })],
  });
  restore = setTransport(t.transport);
  return t;
}

describe('App sources panel', () => {
  it('is absent when no source is enabled and appears (polled) when one is', async () => {
    const t = fakeTransport({
      'GET /api/overview': overview(),
      'GET /api/messages': [],
      'GET /api/sources/jira': sourceView('jira', { tickets: [ticket({ key: 'AA-9' })] }),
      'GET /api/sources': [
        { source: 'linear', enabled: false, site: null },
        { source: 'jira', enabled: true, site: null },
      ],
    });
    restore = setTransport(t.transport);
    render(App);
    expect(await screen.findByText('AA-9')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Sources' })).toBeInTheDocument();
    const paths = t.api().map((c) => `${c.method} ${c.path}`);
    expect(paths).toContain('GET /api/sources/jira');
    expect(paths).not.toContain('GET /api/sources/linear');
  });

  it('shows nothing when GET /api/sources reports nothing enabled', async () => {
    const t = fakeTransport({
      'GET /api/overview': overview(),
      'GET /api/messages': [],
      'GET /api/sources': [
        { source: 'linear', enabled: false, site: null },
        { source: 'jira', enabled: false, site: null },
      ],
    });
    restore = setTransport(t.transport);
    render(App);
    await screen.findByText('Agents & sessions');
    await waitFor(() => expect(t.api().some((c) => c.path === '/api/sources')).toBe(true));
    expect(screen.queryByRole('heading', { name: 'Sources' })).toBeNull();
    expect(t.api().some((c) => c.path?.startsWith('/api/sources/'))).toBe(false);
  });
});

describe('App', () => {
  it('polls overview and messages and renders every panel', async () => {
    const t = boot();
    render(App);
    expect(await screen.findByText('hello there')).toBeInTheDocument();
    expect(screen.getByText('Agents & sessions')).toBeInTheDocument();
    expect(screen.getByText('Activity')).toBeInTheDocument();
    expect(screen.getByText('Decisions')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Tasks' })).toBeInTheDocument();
    const paths = t.api().map((c) => `${c.method} ${c.path}`);
    expect(paths).toContain('GET /api/overview');
    expect(paths).toContain('GET /api/messages?limit=200');
    // Viewing is read-only: only GETs.
    expect(t.api().every((c) => c.method === 'GET')).toBe(true);
  });

  it('clicking the unread chip filters Messages to unread (server-side too)', async () => {
    const t = boot();
    render(App);
    await screen.findByText('hello there');
    await fireEvent.click(screen.getByRole('button', { name: /unread/ }));
    expect(screen.getByLabelText('Message filter')).toHaveValue('unread');
    await waitFor(() => expect(t.api().some((c) => c.path === '/api/messages?limit=200&unread_only=true')).toBe(true));
  });

  it('pause button stops polling and labels itself Resume', async () => {
    const t = boot();
    render(App);
    await screen.findByText('hello there');
    await fireEvent.click(screen.getByRole('button', { name: 'Pause' }));
    expect(screen.getByRole('button', { name: 'Resume' })).toBeInTheDocument();
    const n = t.calls.length;
    await new Promise((r) => setTimeout(r, 50));
    expect(t.calls.length).toBe(n);
  });

  it('the theme toggle stores the choice and sets data-theme', async () => {
    boot();
    render(App);
    await screen.findByText('hello there');
    await fireEvent.click(screen.getByRole('button', { name: 'Toggle theme' }));
    const first = document.documentElement.getAttribute('data-theme');
    expect(first === 'dark' || first === 'light').toBe(true);
    expect(localStorage.getItem('corvus-theme')).toBe(first);
    await fireEvent.click(screen.getByRole('button', { name: 'Toggle theme' }));
    expect(document.documentElement.getAttribute('data-theme')).not.toBe(first);
  });

  it('shows a poll error without crashing when the app returns an error', async () => {
    restore = setTransport(
      fakeTransport({ 'GET /api/': () => ({ status: 500, body: { error: { code: 'internal', message: 'db locked' } } }) }).transport,
    );
    render(App);
    expect(await screen.findByTestId('poll-error')).toHaveTextContent('internal: db locked');
  });

  it('the Connect view opens from the header', async () => {
    const t = boot();
    restore();
    restore = setTransport(
      fakeTransport({
        'GET /api/overview': overview(),
        'GET /api/messages': [],
        app_info: { version: '9', corvus_path: null, cli_installed: true, adapters: [], mcp_snippet: 'm', hook_snippet: 'h' },
      }).transport,
    );
    void t;
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Connect an agent' }));
    expect(await screen.findByText('Corvus 9')).toBeInTheDocument();
  });
});
