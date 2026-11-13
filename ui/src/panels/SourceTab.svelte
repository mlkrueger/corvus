<script lang="ts">
  import { untrack } from 'svelte';
  import { errorText } from '../lib/api';
  import { ago } from '../lib/format';
  import { safeHttpsUrl } from '../lib/links';
  import {
    appendStatus,
    boardColumns,
    canQueue,
    canRemove,
    filterFromForm,
    formFromFilter,
    projectsLabel,
    queueLabel,
    queueTicket,
    removeFromQueue,
    ruleExample,
    SOURCE_LABELS,
    saveFilter,
    suggestions,
    syncSource,
    syncSummary,
  } from '../lib/sources';
  import type { SourceId } from '../lib/types/SourceId';
  import type { SourceTicketView } from '../lib/types/SourceTicketView';
  import type { SourceView } from '../lib/types/SourceView';

  interface Props {
    source: SourceId;
    view: SourceView | null;
    loadError: string | null;
    site: string | null;
    operator: string;
    now: number;
    onReload: () => Promise<void>;
    onView: (v: SourceView) => void;
  }
  let { source, view, loadError, site, operator, now, onReload, onView }: Props = $props();

  const vocab = $derived(projectsLabel(source));
  const columns = $derived(view ? boardColumns(view) : []);

  let busy = $state(false);
  let notice = $state<{ kind: 'ok' | 'error'; text: string } | null>(null);

  async function act(run: () => Promise<string | null>): Promise<void> {
    if (busy) return;
    busy = true;
    notice = null;
    try {
      const text = await run();
      if (text !== null) notice = { kind: 'ok', text };
    } catch (e) {
      notice = { kind: 'error', text: errorText(e) };
    } finally {
      busy = false;
    }
  }

  const add = (t: SourceTicketView) =>
    act(async () => {
      await queueTicket(source, t.key);
      await onReload();
      return `${t.key} queued`;
    });

  const remove = (t: SourceTicketView) =>
    act(async () => {
      await removeFromQueue(operator, t.task_id!);
      await onReload();
      return `${t.key} removed from the queue`;
    });

  const sync = () =>
    act(async () => {
      const r = await syncSource(source);
      await onReload();
      if (!r.ok) throw new Error(`sync failed: ${r.error ?? 'unknown error'}`);
      return syncSummary(r);
    });

  // ---- Filter form: edits survive polls; a clean form follows the server's filter ----
  let form = $state(formFromFilter(null));
  let dirty = $state(false);
  let formErrors = $state<string[]>([]);
  let saving = $state(false);
  const serverKey = $derived(JSON.stringify(view?.filter ?? null));

  $effect(() => {
    void serverKey;
    if (dirty) return;
    untrack(() => (form = formFromFilter(view?.filter ?? null)));
  });

  const suggested = $derived(suggestions(view?.known_statuses ?? [], form.statuses));

  async function save(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (saving) return;
    const parsed = filterFromForm(form);
    if (!parsed.ok) {
      formErrors = parsed.errors;
      return;
    }
    formErrors = [];
    saving = true;
    try {
      const fresh = await saveFilter(source, parsed.filter);
      dirty = false;
      form = formFromFilter(fresh.filter);
      onView(fresh);
      notice = { kind: 'ok', text: 'filter saved' };
    } catch (err) {
      formErrors = [errorText(err)];
    } finally {
      saving = false;
    }
  }
</script>

{#if loadError}<p class="error" role="alert">{loadError}</p>{/if}
{#if view === null}
  {#if !loadError}<p class="empty">loading...</p>{/if}
{:else if !view.enabled}
  <p class="empty">not configured</p>
{:else}
  <div class="source-bar">
    <button type="button" onclick={sync} disabled={busy}>Sync now</button>
    {#if site}<span class="muted">{site}</span>{/if}
    <span class="muted" data-sync="last">
      {view.last_sync_at ? `last sync ${ago(view.last_sync_at, now)}` : 'never synced'}
    </span>
    {#if view.last_error}<span class="error" role="alert" data-sync="error">last sync failed: {view.last_error}</span>{/if}
    {#if notice}
      <span class:error={notice.kind === 'error'} class:muted={notice.kind === 'ok'} role={notice.kind === 'error' ? 'alert' : 'status'} data-notice={notice.kind}>{notice.text}</span>
    {/if}
  </div>

  <div class="board source-board">
    {#each columns as col (col.id)}
      <div class="column" data-column={col.done ? 'source-done' : 'source-status'}>
        <h3>{col.title} <span class="count">{col.tickets.length}</span></h3>
        {#if col.tickets.length === 0}<p class="empty">none</p>{/if}
        <ul class="cards">
          {#each col.tickets as t (t.key)}
            {@const href = safeHttpsUrl(t.url)}
            <li class="card" data-ticket={t.key}>
              <div class="card-title">
                {#if href}<a class="ticket-key" {href} target="_blank" rel="noopener noreferrer">{t.key}</a>{:else}<span class="ticket-key">{t.key}</span>{/if}
                <span class="ticket-summary">{t.summary}</span>
              </div>
              <div class="card-meta">
                <span class="badge prio">P{t.priority}</span>
                <span class="muted">{t.project}</span>
                {#if t.auto_queue}<span class="badge" data-badge="auto">auto</span>{/if}
                {#if t.task_status}<span class="badge task-{t.task_status}" data-badge="task">{t.task_status}</span>{/if}
                {#if t.manual}<span class="badge" data-badge="manual">by hand</span>{/if}
              </div>
              {#if canQueue(t) || canRemove(t)}
                <div class="card-meta">
                  {#if canQueue(t)}
                    <button type="button" disabled={busy} onclick={() => add(t)}>{queueLabel(t)}</button>
                  {/if}
                  {#if canRemove(t)}
                    <button type="button" class="danger" disabled={busy} onclick={() => remove(t)}>Remove from queue</button>
                  {/if}
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      </div>
    {/each}
  </div>

  <form class="filter-form" aria-label="{SOURCE_LABELS[source]} filter" onsubmit={save}>
    <h3>Filter</h3>
    <label class="field">
      <span>Statuses (one per line)</span>
      <textarea rows="5" bind:value={form.statuses} oninput={() => (dirty = true)}></textarea>
    </label>
    {#if suggested.length > 0}
      <div class="suggestions" aria-label="Status suggestions">
        <span class="muted">Add:</span>
        {#each suggested as s (s)}
          <button
            type="button"
            class="suggestion"
            onclick={() => {
              form.statuses = appendStatus(form.statuses, s);
              dirty = true;
            }}>{s}</button>
        {/each}
      </div>
    {/if}
    <label class="field narrow">
      <span>Done window (days, 0 = hide)</span>
      <input type="text" inputmode="numeric" bind:value={form.doneDays} oninput={() => (dirty = true)} />
    </label>
    <label class="field">
      <span>{vocab} (comma-separated, empty = all)</span>
      <input type="text" bind:value={form.projects} oninput={() => (dirty = true)} />
    </label>
    <label class="field">
      <span>Auto-queue rules (one per line, * = any)</span>
      <textarea rows="3" placeholder={ruleExample(source)} bind:value={form.rules} oninput={() => (dirty = true)}></textarea>
    </label>
    <p class="muted hint">Rule format: {vocab.toUpperCase()}: STATUSES, for example <code>{ruleExample(source)}</code></p>
    {#each formErrors as msg, i (i)}<p class="error" role="alert">{msg}</p>{/each}
    <div class="compose-row">
      <button type="submit" disabled={saving}>Save filter</button>
      <span class="muted">Saving runs a sync.</span>
    </div>
  </form>
{/if}
