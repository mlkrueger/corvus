<script lang="ts">
  import { reassignMail } from '../lib/actions';
  import { errorText } from '../lib/api';
  import { ago, leaseExpired, leaseText } from '../lib/format';
  import type { AgentStatus } from '../lib/types/AgentStatus';
  import type { OverviewSession } from '../lib/types/OverviewSession';
  import type { TaskView } from '../lib/types/TaskView';

  interface Props {
    sessions: OverviewSession[];
    agents?: AgentStatus[];
    tasks: TaskView[];
    now: number;
    onChanged?: () => void;
  }
  let { sessions, agents = [], tasks, now, onChanged = () => {} }: Props = $props();

  let showAll = $state(false);

  const LIVE = new Set(['active', 'idle', 'working']);
  const shown = $derived(showAll ? sessions : sessions.filter((s) => LIVE.has(s.presence)));
  /** Agents with mail nothing can deliver: no open session, no waiter, no watcher. */
  const stranded = $derived(agents.filter((a) => a.unread_direct > 0 && !a.reachable));

  function taskTitle(id: string | null): string {
    if (!id) return '';
    return tasks.find((t) => t.id === id)?.title ?? id;
  }

  function metadataText(m: unknown): string {
    if (m === null || typeof m !== 'object' || Object.keys(m).length === 0) return '';
    return JSON.stringify(m);
  }

  /** The agent's newest conversation: its tool and working directory. */
  function conversationText(name: string): string {
    const c = agents.find((a) => a.name === name)?.conversations[0];
    if (!c) return '';
    return c.directory ? `${c.tool} · ${c.directory}` : c.tool;
  }

  // Reassign: one target selection per stranded agent.
  let targets = $state<Record<string, string>>({});
  let busy = $state<string | null>(null);
  let reassignError = $state<string | null>(null);

  function targetsFor(name: string): string[] {
    return agents.filter((a) => a.name !== name).map((a) => a.name);
  }

  async function reassign(e: SubmitEvent, from: string): Promise<void> {
    e.preventDefault();
    const to = targets[from] ?? '';
    if (to === '' || busy !== null) return;
    busy = from;
    reassignError = null;
    try {
      await reassignMail(from, to);
      onChanged();
    } catch (err) {
      reassignError = errorText(err);
    } finally {
      busy = null;
    }
  }
</script>

<section class="panel" aria-labelledby="sessions-h">
  <div class="panel-head">
    <h2 id="sessions-h">Agents &amp; sessions</h2>
    <label class="inline"><input type="checkbox" bind:checked={showAll} /> show offline / ended</label>
  </div>
  {#if stranded.length > 0}
    <ul class="rows stranded" aria-label="Agents with undeliverable mail">
      {#each stranded as a (a.name)}
        <li class="row" data-stranded={a.name}>
          <div class="row-main">
            <strong class="agent">{a.name}</strong>
            <span class="badge unreachable">not reachable</span>
            <span class="badge unread">{a.unread_direct} unread</span>
          </div>
          {#if conversationText(a.name)}
            <div class="row-sub muted conversation">{conversationText(a.name)}</div>
          {/if}
          <div class="row-sub muted">Its mail waits until that conversation is resumed.</div>
          <form class="compose-row" onsubmit={(e) => reassign(e, a.name)} aria-label="Reassign mail of {a.name}">
            <label class="inline">
              Reassign unread mail to
              <select aria-label="Reassign {a.name} to" bind:value={targets[a.name]}>
                <option value="">choose an agent</option>
                {#each targetsFor(a.name) as t (t)}<option value={t}>{t}</option>{/each}
              </select>
            </label>
            <button type="submit" disabled={busy !== null || (targets[a.name] ?? '') === ''}>Reassign</button>
          </form>
        </li>
      {/each}
    </ul>
    {#if reassignError}<p class="error" role="alert">{reassignError}</p>{/if}
  {/if}
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
          {#if conversationText(s.agent_name)}
            <div class="row-sub muted conversation">{conversationText(s.agent_name)}</div>
          {/if}
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
