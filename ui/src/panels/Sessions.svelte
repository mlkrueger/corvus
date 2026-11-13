<script lang="ts">
  import { ago, leaseExpired, leaseText } from '../lib/format';
  import type { OverviewSession } from '../lib/types/OverviewSession';
  import type { TaskView } from '../lib/types/TaskView';

  interface Props {
    sessions: OverviewSession[];
    tasks: TaskView[];
    now: number;
  }
  let { sessions, tasks, now }: Props = $props();

  let showAll = $state(false);

  const LIVE = new Set(['active', 'idle', 'working']);
  const shown = $derived(showAll ? sessions : sessions.filter((s) => LIVE.has(s.presence)));

  function taskTitle(id: string | null): string {
    if (!id) return '';
    return tasks.find((t) => t.id === id)?.title ?? id;
  }

  function metadataText(m: unknown): string {
    if (m === null || typeof m !== 'object' || Object.keys(m).length === 0) return '';
    return JSON.stringify(m);
  }
</script>

<section class="panel" aria-labelledby="sessions-h">
  <div class="panel-head">
    <h2 id="sessions-h">Agents &amp; sessions</h2>
    <label class="inline"><input type="checkbox" bind:checked={showAll} /> show offline / ended</label>
  </div>
  {#if shown.length === 0}
    <p class="empty">No live sessions.</p>
  {:else}
    <ul class="rows">
      {#each shown as s (s.id)}
        <li class="row session" data-session={s.agent_name}>
          <div class="row-main">
            <strong class="agent">{s.agent_name}</strong>
            <span class="badge presence-{s.presence}">{s.presence}</span>
            <span class="muted">{s.client_name}</span>
            {#if s.unread > 0}<span class="badge unread">{s.unread} unread</span>{/if}
          </div>
          <div class="row-sub muted">
            last seen {ago(s.last_seen_at, now)}
            {#if s.ended_at}<span>· ended {ago(s.ended_at, now)}{s.end_reason ? ` (${s.end_reason})` : ''}</span>{/if}
          </div>
          {#if s.current_task}
            <div class="row-sub">
              task: <span class="task-ref">{taskTitle(s.current_task)}</span>
              {#if s.lease_expires_at}
                <span class="lease" class:warn={leaseExpired(s.lease_expires_at, now)}>{leaseText(s.lease_expires_at, now)}</span>
              {/if}
            </div>
          {/if}
          {#if metadataText(s.metadata)}
            <div class="row-sub muted meta">{metadataText(s.metadata)}</div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
