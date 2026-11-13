// Ticket links render as links only if they are https (system/05 "Untrusted text"). Used by plan 0008 (Sources).

/** Returns the normalised URL when `value` is a valid https URL, otherwise null. */
export function safeHttpsUrl(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  try {
    const url = new URL(value);
    return url.protocol === 'https:' ? url.href : null;
  } catch {
    return null;
  }
}
