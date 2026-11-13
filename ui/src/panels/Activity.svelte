<script lang="ts">
  import { ago } from '../lib/format';
  import type { EventView } from '../lib/types/EventView';

  interface Props {
    items: EventView[];
    now: number;
  }
  let { items, now }: Props = $props();
</script>

<section class="panel" aria-labelledby="activity-h">
  <div class="panel-head"><h2 id="activity-h">Activity</h2></div>
  {#if items.length === 0}
    <p class="empty">No activity yet.</p>
  {:else}
    <ul class="rows scroll">
      {#each items as e (e.id)}
        <li class="row event">
          <div class="row-main">
            <span class="badge">{e.kind}</span>
            {#if e.agent_name}<strong class="agent">{e.agent_name}</strong>{/if}
            <span class="muted">{ago(e.created_at, now)}</span>
          </div>
          <div class="row-sub">{e.summary}</div>
        </li>
      {/each}
    </ul>
  {/if}
</section>
