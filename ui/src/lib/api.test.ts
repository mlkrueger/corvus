import { afterEach, describe, expect, it } from 'vitest';
import { fakeTransport } from '../fixtures';
import { ApiError, appInfo, request, setTransport } from './api';

let restore: () => void = () => {};
afterEach(() => restore());

describe('request', () => {
  it('sends method, path and body over the api command and returns the body', async () => {
    const { transport, calls } = fakeTransport({ 'POST /api/x': { ok: true } });
    restore = setTransport(transport);
    const out = await request<{ ok: boolean }>('POST', '/api/x', { a: 1 });
    expect(out).toEqual({ ok: true });
    expect(calls[0]).toMatchObject({ cmd: 'api', method: 'POST', path: '/api/x', body: { a: 1 } });
  });

  it('sends a null body when none is given', async () => {
    const { transport, calls } = fakeTransport({ 'GET /api/x': null });
    restore = setTransport(transport);
    expect(await request('GET', '/api/x')).toBeNull();
    expect(calls[0]!.body).toBeNull();
  });

  it('maps status >= 400 to a typed ApiError', async () => {
    const { transport } = fakeTransport({
      'GET /api/x': () => ({ status: 409, body: { error: { code: 'conflict', message: 'already claimed' } } }),
    });
    restore = setTransport(transport);
    const err = await request('GET', '/api/x').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 409, code: 'conflict', message: 'already claimed' });
  });

  it('tolerates an error response without the standard body', async () => {
    const { transport } = fakeTransport({ 'GET /api/x': () => ({ status: 500, body: null }) });
    restore = setTransport(transport);
    await expect(request('GET', '/api/x')).rejects.toMatchObject({ status: 500, code: 'unknown' });
  });

  it('maps an IPC failure to ApiError status 0', async () => {
    restore = setTransport(async () => {
      throw new Error('ipc down');
    });
    await expect(request('GET', '/api/x')).rejects.toMatchObject({ status: 0, code: 'ipc_error', message: 'ipc down' });
  });

  it('rejects a malformed response', async () => {
    restore = setTransport(async () => 'nope');
    await expect(request('GET', '/api/x')).rejects.toMatchObject({ status: 0, code: 'ipc_error' });
  });
});

describe('appInfo', () => {
  it('returns app_info, or a status-0 ApiError when the command fails', async () => {
    const info = { version: '1', corvus_path: null, cli_installed: false, adapters: [], mcp_snippet: '', hook_snippet: '' };
    restore = setTransport(fakeTransport({ app_info: info }).transport);
    expect(await appInfo()).toEqual(info);
    restore();
    restore = setTransport(fakeTransport({}).transport);
    await expect(appInfo()).rejects.toMatchObject({ status: 0 });
  });
});
