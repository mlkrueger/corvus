// Small pure formatting helpers (time, lease countdown).

export function toMs(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? null : ms;
}

function span(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ${String(m % 60).padStart(2, '0')}m`;
  return `${Math.floor(h / 24)}d`;
}

/** "5s ago", "3m 02s ago"; "-" when unknown. */
export function ago(iso: string | null | undefined, now: number): string {
  const ms = toMs(iso);
  if (ms === null) return '-';
  return `${span((now - ms) / 1000)} ago`;
}

/** "12m 03s left" or "expired 2m 00s ago"; "" when there is no lease. */
export function leaseText(iso: string | null | undefined, now: number): string {
  const ms = toMs(iso);
  if (ms === null) return '';
  const diff = (ms - now) / 1000;
  return diff >= 0 ? `${span(diff)} left` : `expired ${span(-diff)} ago`;
}

export function leaseExpired(iso: string | null | undefined, now: number): boolean {
  const ms = toMs(iso);
  return ms !== null && ms < now;
}

export function shortId(id: string): string {
  return id.slice(0, 8);
}
