// Sources panel helpers: board layout (pure) and the operator actions (system/05 "Sources panel", 07 "HTTP surface").
import { cancelTask } from './actions';
import { request } from './api';
import type { AutoQueueRule } from './types/AutoQueueRule';
import type { QueueTicket } from './types/QueueTicket';
import type { SourceFilter } from './types/SourceFilter';
import type { SourceId } from './types/SourceId';
import type { SourceSummary } from './types/SourceSummary';
import type { SourceTicketView } from './types/SourceTicketView';
import type { SourceView } from './types/SourceView';
import type { SyncResult } from './types/SyncResult';
import type { TaskView } from './types/TaskView';
import { formatRules, parseRules } from './sourceRules';

export const REMOVE_REASON = 'removed from queue on the dashboard';

export const SOURCE_LABELS: Record<SourceId, string> = { jira: 'Jira', linear: 'Linear' };

/** The tracker's word for what the filter and rules key on: Jira projects, Linear teams. */
export function projectsLabel(source: SourceId): string {
  return source === 'linear' ? 'Teams' : 'Projects';
}

/** An example rule line in the source's own vocabulary. */
export function ruleExample(source: SourceId): string {
  return source === 'linear' ? 'ENG, OPS: Todo, In Progress' : 'AA, LAI: To Do, In Progress';
}

export interface BoardColumn {
  id: string;
  title: string;
  /** True for the "Done in last N days" column. */
  done: boolean;
  tickets: SourceTicketView[];
}

const same = (a: string, b: string): boolean => a.trim().toLowerCase() === b.trim().toLowerCase();

/**
 * One column per filter status (tickets in that status) plus "Done in last N days" (status category done; hidden
 * when N is 0). A done ticket is shown in the done column only, unless the window is 0.
 */
export function boardColumns(view: SourceView): BoardColumn[] {
  const filter = view.filter;
  if (!filter) return [];
  const showDone = filter.done_days > 0;
  const columns: BoardColumn[] = filter.statuses.map((name, i) => ({
    id: `status-${i}`,
    title: name,
    done: false,
    tickets: view.tickets.filter((t) => same(t.status, name) && !(showDone && t.status_category === 'done')),
  }));
  if (showDone) {
    const n = filter.done_days;
    columns.push({
      id: 'done',
      title: `Done in last ${n === 1 ? 'day' : `${n} days`}`,
      done: true,
      tickets: view.tickets.filter((t) => t.status_category === 'done'),
    });
  }
  return columns;
}

/** A ticket's task is live (open or claimed), so it cannot be queued again. */
export function hasLiveTask(t: SourceTicketView): boolean {
  return t.task_status === 'open' || t.task_status === 'claimed';
}

export function canQueue(t: SourceTicketView): boolean {
  return !hasLiveTask(t) && t.status_category !== 'done';
}

/** "Queue now" for auto-queue tickets that have no task yet, otherwise "Add to queue". */
export function queueLabel(t: SourceTicketView): string {
  return t.auto_queue && t.task_id === null ? 'Queue now' : 'Add to queue';
}

/** "Remove from queue" cancels the ticket's task, only while it is unclaimed. */
export function canRemove(t: SourceTicketView): boolean {
  return t.task_status === 'open' && t.task_id !== null;
}

const base = (source: SourceId): string => `/api/sources/${encodeURIComponent(source)}`;

export function fetchSources(): Promise<SourceSummary[]> {
  return request('GET', '/api/sources');
}

export function fetchSource(source: SourceId): Promise<SourceView> {
  return request<SourceView>('GET', base(source));
}

export function queueTicket(source: SourceId, key: string): Promise<TaskView> {
  const body: QueueTicket = { key };
  return request<TaskView>('POST', `${base(source)}/queue`, body);
}

export function removeFromQueue(operator: string, taskId: string): Promise<void> {
  return cancelTask(operator, taskId, REMOVE_REASON);
}

export function syncSource(source: SourceId): Promise<SyncResult> {
  return request<SyncResult>('POST', `${base(source)}/sync`);
}

export function saveFilter(source: SourceId, filter: SourceFilter): Promise<SourceView> {
  return request<SourceView>('PUT', `${base(source)}/filter`, filter);
}

export function syncSummary(r: SyncResult): string {
  const s = r.stats;
  if (!s) return 'synced';
  return `synced ${s.tickets} tickets, ${s.tasks_created} tasks created, ${s.comments_delivered} comments delivered`;
}

// ---- Filter form <-> SourceFilter ----

export interface FilterForm {
  statuses: string;
  doneDays: string;
  projects: string;
  rules: string;
}

export function formFromFilter(f: SourceFilter | null): FilterForm {
  if (!f) return { statuses: '', doneDays: '0', projects: '', rules: '' };
  return {
    statuses: f.statuses.join('\n'),
    doneDays: String(f.done_days),
    projects: f.projects.join(', '),
    rules: formatRules(f.auto_queue),
  };
}

export type FilterParse = { ok: true; filter: SourceFilter } | { ok: false; errors: string[] };

const lines = (s: string): string[] =>
  s
    .split(/\r?\n/)
    .map((x) => x.trim())
    .filter((x) => x !== '');

/** Form text to the PUT body. Syntax problems are reported here; value validation is the server's (422). */
export function filterFromForm(form: FilterForm): FilterParse {
  const errors: string[] = [];
  const days = form.doneDays.trim();
  if (!/^\d+$/.test(days)) errors.push('Done window must be a whole number of days.');
  const parsed = parseRules(form.rules);
  for (const e of parsed.errors) errors.push(`Rule line ${e.line}: ${e.message}`);
  if (errors.length > 0) return { ok: false, errors };
  const auto_queue: AutoQueueRule[] = parsed.rules.map((r) => ({ projects: r.projects, statuses: r.statuses }));
  return {
    ok: true,
    filter: {
      statuses: lines(form.statuses),
      done_days: Number(days),
      projects: form.projects
        .split(',')
        .map((x) => x.trim())
        .filter((x) => x !== ''),
      auto_queue,
    },
  };
}

/** Known statuses not yet listed in the form's statuses text (case-insensitive). */
export function suggestions(known: string[], statusesText: string): string[] {
  const have = lines(statusesText);
  return known.filter((k, i) => known.indexOf(k) === i && !have.some((h) => same(h, k)));
}

/** Appends a status as a new line. */
export function appendStatus(statusesText: string, status: string): string {
  const t = statusesText.replace(/\s+$/, '');
  return t === '' ? status : `${t}\n${status}`;
}
