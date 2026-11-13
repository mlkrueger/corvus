<script lang="ts">
  import { cancelTask } from '../lib/actions';
  import { errorText } from '../lib/api';
  import { leaseExpired, leaseText, shortId } from '../lib/format';
  import type { TaskColumns } from '../lib/types/TaskColumns';
  import type { TaskView } from '../lib/types/TaskView';

  interface Props {
    tasks: TaskColumns;
    operator: string;
    now: number;
    onChanged: () => void;
  }
  let { tasks, operator, now, onChanged }: Props = $props();

  const columns = $derived([
    { key: 'open', label: 'Open', items: tasks.open },
    { key: 'claimed', label: 'Claimed', items: tasks.claimed },
    { key: 'done', label: 'Done', items: tasks.done },
    { key: 'cancelled', label: 'Cancelled', items: tasks.cancelled },
  ]);

  let cancelling = $state<string | null>(null);
  let reason = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);

  function startCancel(t: TaskView): void {
    cancelling = t.id;
    reason = '';
    error = null;
  }

  async function confirmCancel(e: SubmitEvent, t: TaskView): Promise<void> {
    e.preventDefault();
    if (busy) return;
    busy = true;
    error = null;
    try {
      await cancelTask(operator, t.id, reason);
      cancelling = null;
      onChanged();
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<section class="panel" aria-labelledby="tasks-h">
  <div class="panel-head"><h2 id="tasks-h">Tasks</h2></div>
  <div class="board">
    {#each columns as col (col.key)}
      <div class="column" data-column={col.key}>
        <h3>{col.label} <span class="count">{col.items.length}</span></h3>
        {#if col.items.length === 0}
          <p class="empty">none</p>
        {/if}
        <ul class="cards">
          {#each col.items as t (t.id)}
            <li class="card" data-task={t.id}>
              <div class="card-title">{t.title}</div>
              <div class="card-meta">
                <span class="badge prio">P{t.priority}</span>
                <span class="muted">{shortId(t.id)}</span>
                {#if t.blocked}<span class="badge warn" data-flag="blocked">blocked</span>{/if}
                {#if t.blocked_by_cancelled}<span class="badge warn" data-flag="blocked-by-cancelled">blocked by cancelled</span>{/if}
              </div>
              {#if t.holder}
                <div class="card-meta">
                  held by <strong class="agent">{t.holder.agent_name}</strong>
                  <span class="lease" class:warn={leaseExpired(t.holder.lease_expires_at, now)}>{leaseText(t.holder.lease_expires_at, now)}</span>
                </div>
              {/if}
              {#if t.recipient_agent}<div class="card-meta muted">for {t.recipient_agent}</div>{/if}
              {#if t.status === 'cancelled' && t.cancel_reason}<div class="card-meta muted">reason: {t.cancel_reason}</div>{/if}
              {#if t.status === 'done' && t.result_summary}<div class="card-meta muted">{t.result_summary}</div>{/if}
              {#if t.status === 'open' || t.status === 'claimed'}
                {#if cancelling === t.id}
                  <form class="cancel-form" onsubmit={(e) => confirmCancel(e, t)} aria-label="Cancel task {t.title}">
                    <input type="text" aria-label="Cancel reason" placeholder="Reason (optional)" bind:value={reason} />
                    <div class="compose-row">
                      <button type="submit" class="danger" disabled={busy}>Confirm cancel</button>
                      <button type="button" onclick={() => (cancelling = null)}>Keep</button>
                    </div>
                    {#if error}<span class="error" role="alert">{error}</span>{/if}
                  </form>
                {:else}
                  <div class="card-meta">
                    <button type="button" class="link-button danger" onclick={() => startCancel(t)}>Cancel</button>
                  </div>
                {/if}
              {/if}
            </li>
          {/each}
        </ul>
      </div>
    {/each}
  </div>
</section>
