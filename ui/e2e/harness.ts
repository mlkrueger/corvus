// Test-only harness (not part of the shipped bundle): a throwaway `corvus serve`, a static server for ui/dist
// that proxies /api/* to it with the bearer token (same origin: the API sends no CORS headers) and sends the app's
// CSP, and a `corvus` CLI wrapper for seeding. Nothing here is imported by src/.
import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
export const UI_DIR = resolve(here, '..');
export const DIST = join(UI_DIR, 'dist');
const REPO = resolve(UI_DIR, '..');
export const CORVUS_BIN = process.env.CORVUS_BIN ?? join(REPO, 'target', 'debug', 'corvus');

const MIME: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
};

/** The app's CSP from tauri.conf.json, with the Tauri IPC origins in connect-src replaced for the browser. */
export function browserCsp(): string {
  const conf = JSON.parse(readFileSync(join(REPO, 'crates', 'corvus-desktop', 'tauri.conf.json'), 'utf8')) as {
    app: { security: { csp: string } };
  };
  const csp = conf.app.security.csp;
  const replaced = csp.replace(/connect-src [^;]*/, "connect-src 'self'");
  if (replaced === csp || !replaced.includes("connect-src 'self'")) throw new Error('connect-src not found in tauri.conf.json csp');
  return replaced;
}

function freePort(): Promise<number> {
  return new Promise((res, rej) => {
    const s = net.createServer();
    s.once('error', rej);
    s.listen(0, '127.0.0.1', () => {
      const { port } = s.address() as net.AddressInfo;
      s.close(() => res(port));
    });
  });
}

export interface ProxiedRequest {
  method: string;
  path: string;
}

export class Harness {
  constructor(
    readonly url: string,
    readonly apiUrl: string,
    private readonly token: string,
    readonly requests: ProxiedRequest[],
    readonly home: string,
    readonly env: NodeJS.ProcessEnv,
    private readonly serve: ChildProcess,
    private readonly web: http.Server,
  ) {}

  /** Run the `corvus` CLI against the throwaway database. Returns stdout; throws on a non-zero exit. */
  cli(...args: string[]): string {
    return execFileSync(CORVUS_BIN, args, { env: this.env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  }

  cliJson<T = Record<string, unknown>>(...args: string[]): T {
    const lines = this.cli(...args).trim().split('\n');
    return JSON.parse(lines[lines.length - 1]!) as T;
  }

  /** Direct (not proxied, not counted) call to the throwaway `corvus serve`, for assertions. */
  async api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
    const r = await fetch(this.apiUrl + path, {
      method,
      headers: { authorization: `Bearer ${this.token}`, ...(body === undefined ? {} : { 'content-type': 'application/json' }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!r.ok) throw new Error(`${method} ${path} -> ${r.status} ${await r.text()}`);
    return (await r.json()) as T;
  }

  /** Counts proxied `/api` requests since the last call to `resetRequests`. */
  resetRequests(): void {
    this.requests.length = 0;
  }

  async stop(): Promise<void> {
    const closed = new Promise<void>((r) => this.web.close(() => r()));
    this.web.closeAllConnections();
    await closed;
    this.serve.kill('SIGTERM');
    await new Promise<void>((r) => {
      if (this.serve.exitCode !== null) return r();
      this.serve.once('exit', () => r());
      setTimeout(r, 3000);
    });
    rmSync(this.home, { recursive: true, force: true });
  }
}

export async function startHarness(): Promise<Harness> {
  if (!existsSync(CORVUS_BIN)) throw new Error(`corvus binary missing at ${CORVUS_BIN}; run \`cargo build -p corvus-cli\``);
  if (!existsSync(join(DIST, 'index.html'))) throw new Error('ui/dist missing; run `npm run build`');

  const home = mkdtempSync(join(tmpdir(), 'corvus-e2e-'));
  const token = randomBytes(32).toString('hex');
  const apiPort = await freePort();
  const env: NodeJS.ProcessEnv = {
    PATH: process.env.PATH,
    HOME: home,
    CORVUS_DATABASE_URL: join(home, 'corvus.db'),
    CORVUS_STATE_DIR: join(home, 'state'),
    CORVUS_API_TOKEN: token,
    CORVUS_HTTP_PORT: String(apiPort),
    CORVUS_OPERATOR_NAME: 'operator',
  };

  const serve = spawn(CORVUS_BIN, ['serve'], { env, stdio: ['ignore', 'ignore', 'pipe'] });
  let serveErr = '';
  serve.stderr?.on('data', (d: Buffer) => (serveErr += d.toString()));
  const api = `http://127.0.0.1:${apiPort}`;
  const deadline = Date.now() + 20_000;
  for (;;) {
    try {
      const r = await fetch(`${api}/api/health`, { headers: { authorization: `Bearer ${token}` } });
      if (r.ok) break;
    } catch {
      // not up yet
    }
    if (serve.exitCode !== null || Date.now() > deadline) throw new Error(`corvus serve did not start: ${serveErr}`);
    await new Promise((r) => setTimeout(r, 100));
  }

  const csp = browserCsp();
  const requests: ProxiedRequest[] = [];
  const web = http.createServer(async (req, res) => {
    const url = new URL(req.url ?? '/', 'http://localhost');
    const common = { 'content-security-policy': csp, 'cache-control': 'no-store', 'x-content-type-options': 'nosniff' };
    try {
      if (url.pathname.startsWith('/api/')) {
        requests.push({ method: req.method ?? 'GET', path: url.pathname + url.search });
        const chunks: Buffer[] = [];
        for await (const c of req) chunks.push(c as Buffer);
        const body = Buffer.concat(chunks);
        const headers: Record<string, string> = { authorization: `Bearer ${token}` };
        if (req.headers['content-type']) headers['content-type'] = String(req.headers['content-type']);
        const upstream = await fetch(api + url.pathname + url.search, {
          method: req.method,
          headers,
          body: body.length > 0 && req.method !== 'GET' && req.method !== 'HEAD' ? body : undefined,
        });
        res.writeHead(upstream.status, { ...common, 'content-type': upstream.headers.get('content-type') ?? 'application/json' });
        res.end(Buffer.from(await upstream.arrayBuffer()));
        return;
      }
      const rel = normalize(decodeURIComponent(url.pathname)).replace(/^([/\\])+/, '');
      const file = resolve(DIST, rel === '' ? 'index.html' : rel);
      if (!file.startsWith(DIST + '/') && file !== DIST) {
        res.writeHead(403, common).end();
        return;
      }
      if (!existsSync(file)) {
        res.writeHead(404, common).end('not found');
        return;
      }
      res.writeHead(200, { ...common, 'content-type': MIME[extname(file)] ?? 'application/octet-stream' });
      res.end(readFileSync(file));
    } catch (e) {
      res.writeHead(502, common).end(String(e));
    }
  });
  await new Promise<void>((r) => web.listen(0, '127.0.0.1', r));
  const webPort = (web.address() as net.AddressInfo).port;

  return new Harness(`http://127.0.0.1:${webPort}`, api, token, requests, home, env, serve, web);
}

/**
 * Init script defining the test-only Tauri IPC shim: `invoke('api', {method, path, body})` becomes a same-origin
 * fetch to the static server's /api proxy and resolves `{status, body}` exactly as the desktop app's command does.
 * `@tauri-apps/api/core`'s `invoke` calls `window.__TAURI_INTERNALS__.invoke(cmd, args, options)`.
 */
export const IPC_SHIM = `
(() => {
  const APP_INFO = {
    version: '0.0.0-e2e',
    corvus_path: '/opt/corvus/bin/corvus',
    cli_installed: false,
    adapters: [{ name: 'claude-code', installed: true }, { name: 'codex', installed: false }],
    mcp_snippet: '{"mcpServers":{"corvus":{"command":"/opt/corvus/bin/corvus","args":["mcp"]}}}',
    hook_snippet: 'corvus hook --claude',
  };
  // Sources can't be configured against a fake tracker from the browser harness, so a test may define
  // window.__sourcesMock = { summaries, views, responses } (canned SourceSummary[] / SourceView by source id /
  // {'METHOD /path': {status, body}} overrides). Without it, /api/sources* passes through to the real server.
  // Every request answered from the mock is recorded in window.__sourceRequests.
  const answerSources = (method, path, body) => {
    const m = window.__sourcesMock;
    window.__sourceRequests = window.__sourceRequests || [];
    window.__sourceRequests.push({ method, path, body });
    const canned = m.responses && m.responses[method + ' ' + path];
    if (canned) return canned;
    const parts = path.split('?')[0].split('/').filter(Boolean); // api, sources, {source}, {action}
    if (method === 'GET' && parts.length === 2) return { status: 200, body: m.summaries };
    const view = m.views && m.views[parts[2]];
    if (!view) return { status: 404, body: { error: { code: 'not_found', message: 'no such source' } } };
    if (method === 'GET' && parts.length === 3) return { status: 200, body: view };
    if (method === 'PUT' && parts[3] === 'filter') {
      m.views[parts[2]] = Object.assign({}, view, { filter: body });
      return { status: 200, body: m.views[parts[2]] };
    }
    return { status: 500, body: { error: { code: 'internal', message: 'no canned response for ' + method + ' ' + path } } };
  };
  let launchAtLogin = true;
  window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      if (cmd === 'api' && window.__sourcesMock && args.path.startsWith('/api/sources')) {
        return answerSources(args.method, args.path, args.body);
      }
      if (cmd === 'api') {
        const hasBody = args.body !== null && args.body !== undefined;
        const res = await fetch(args.path, {
          method: args.method,
          headers: hasBody ? { 'content-type': 'application/json' } : {},
          body: hasBody ? JSON.stringify(args.body) : undefined,
        });
        const text = await res.text();
        let body = null;
        try { body = text ? JSON.parse(text) : null; } catch { body = null; }
        return { status: res.status, body };
      }
      if (cmd === 'app_info') return APP_INFO;
      if (cmd === 'check_update') return { enabled: false, current: APP_INFO.version, available: null, error: null };
      if (cmd === 'get_launch_at_login') return launchAtLogin;
      if (cmd === 'set_launch_at_login') { launchAtLogin = Boolean(args.enabled); return launchAtLogin; }
      throw new Error('unknown command: ' + cmd);
    },
    transformCallback: () => 0,
    unregisterCallback: () => {},
    convertFileSrc: (p) => p,
  };
})();
`;
