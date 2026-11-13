<script lang="ts">
  import { ago } from '../lib/format';
  import type { DecisionView } from '../lib/types/DecisionView';

  interface Props {
    decisions: DecisionView[];
    now: number;
  }
  let { decisions, now }: Props = $props();
</script>

<section class="panel" aria-labelledby="decisions-h">
  <div class="panel-head"><h2 id="decisions-h">Decisions</h2></div>
  {#if decisions.length === 0}
    <p class="empty">No decisions recorded.</p>
  {:else}
    <ul class="rows scroll">
      {#each decisions as d (d.id)}
        <li class="row decision">
          <div class="row-main">
            <strong>{d.title}</strong>
            <span class="muted">{d.decided_by} · {ago(d.created_at, now)}</span>
          </div>
          <div class="row-sub body">{d.decision}</div>
          {#if d.rationale}<div class="row-sub muted body">{d.rationale}</div>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
