// THE data layer: the only module that touches Tauri IPC (system/05). Everything else calls `request`.
import { invoke } from '@tauri-apps/api/core';

export interface ApiResponse {
  status: number;
  body: unknown;
}

/** Mirrors Tauri's `invoke(cmd, args)`; tests and the e2e shim substitute it. */
export type Transport = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

const tauriTransport: Transport = (cmd, args) => invoke(cmd, args);

let transport: Transport = tauriTransport;

/** Replace the transport (unit tests). Returns a function that restores the previous one. */
export function setTransport(next: Transport): () => void {
  const previous = transport;
  transport = next;
  return () => {
    transport = previous;
  };
}

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
  }
}

function isResponse(value: unknown): value is ApiResponse {
  return typeof value === 'object' && value !== null && typeof (value as ApiResponse).status === 'number';
}

function errorFrom(status: number, body: unknown): ApiError {
  const err = (body as { error?: { code?: unknown; message?: unknown } } | null)?.error;
  const code = typeof err?.code === 'string' ? err.code : 'unknown';
  const message = typeof err?.message === 'string' ? err.message : `request failed with status ${status}`;
  return new ApiError(status, code, message);
}

/** Calls the HTTP route table through IPC. Status >= 400 throws `ApiError`; an IPC failure throws status 0. */
export async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: unknown;
  try {
    res = await transport('api', { method, path, body: body ?? null });
  } catch (e) {
    throw new ApiError(0, 'ipc_error', e instanceof Error ? e.message : String(e));
  }
  if (!isResponse(res)) throw new ApiError(0, 'ipc_error', 'malformed response from the app');
  if (res.status >= 400) throw errorFrom(res.status, res.body);
  return res.body as T;
}

export interface AdapterInfo {
  name: string;
  installed: boolean;
}

export interface AppInfo {
  version: string;
  corvus_path: string | null;
  cli_installed: boolean;
  adapters: AdapterInfo[];
  mcp_snippet: string;
  hook_snippet: string;
}

/** App command `app_info`. Rejects (ApiError status 0) when the command is unavailable. */
export async function appInfo(): Promise<AppInfo> {
  try {
    return (await transport('app_info')) as AppInfo;
  } catch (e) {
    throw new ApiError(0, 'ipc_error', e instanceof Error ? e.message : String(e));
  }
}

/** App command `install_cli`: links the bundled binary into the user's PATH. */
export async function installCli(): Promise<void> {
  try {
    await transport('install_cli');
  } catch (e) {
    throw new ApiError(0, 'ipc_error', e instanceof Error ? e.message : String(e));
  }
}

/** A fresh idempotency key; every operator action generates its own per click. */
export function newKey(): string {
  return crypto.randomUUID();
}

export function errorText(e: unknown): string {
  if (e instanceof ApiError) return e.status === 0 ? `app error: ${e.message}` : `${e.code}: ${e.message}`;
  return e instanceof Error ? e.message : String(e);
}
