import { test as base, expect, type Page } from '@playwright/test';
import { Harness, IPC_SHIM, startHarness } from './harness';

export const HOSTILE_IMG = '<img src=x onerror=window.__pwned=1>';
export const HOSTILE_SCRIPT = '<script>window.__pwned=1</script>';

export interface Probe {
  violations: () => Promise<string[]>;
  consoleErrors: string[];
}

/** Init script run before any page script: records CSP violations. */
const PROBE = `
window.__csp = [];
document.addEventListener('securitypolicyviolation', (e) => {
  window.__csp.push(e.violatedDirective + ' ' + e.blockedURI);
});
`;

export const test = base.extend<{ h: Harness; probe: Probe }>({
  // eslint-disable-next-line no-empty-pattern
  h: async ({}, use) => {
    const h = await startHarness();
    await use(h);
    await h.stop();
  },
  // Every test gets the IPC shim and the CSP-violation recorder, whether or not it asks for `probe`.
  page: async ({ page }, use) => {
    await page.addInitScript(IPC_SHIM);
    await page.addInitScript(PROBE);
    await use(page);
  },
  probe: async ({ page }, use) => {
    const consoleErrors: string[] = [];
    page.on('console', (m) => {
      if (m.type() === 'error') consoleErrors.push(m.text());
    });
    page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
    await use({
      consoleErrors,
      violations: () => page.evaluate(() => (window as unknown as { __csp: string[] }).__csp),
    });
  },
});

export { expect };

export async function open(page: Page, h: Harness): Promise<void> {
  await page.goto(h.url + '/');
  await expect(page.getByRole('heading', { name: 'Corvus', level: 1 })).toBeVisible();
}

/** Waits until at least `n` overview polls have been proxied since the last reset. */
export async function waitForPolls(h: Harness, n: number, timeoutMs = 15_000): Promise<void> {
  await expect
    .poll(() => h.requests.filter((r) => r.path.startsWith('/api/overview')).length, { timeout: timeoutMs })
    .toBeGreaterThanOrEqual(n);
}
