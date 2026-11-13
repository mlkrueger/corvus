<script lang="ts">
  import type { SourcesState } from '../lib/sources.svelte';
  import { SOURCE_LABELS } from '../lib/sources';
  import type { SourceId } from '../lib/types/SourceId';
  import SourceTab from './SourceTab.svelte';

  interface Props {
    sources: SourcesState;
    operator: string;
    now: number;
  }
  let { sources, operator, now }: Props = $props();

  const enabled = $derived(sources.enabled);
  let chosen = $state<SourceId | null>(null);
  const active = $derived<SourceId | null>(chosen !== null && enabled.includes(chosen) ? chosen : (enabled[0] ?? null));
</script>

{#if active !== null}
  <section class="panel" aria-labelledby="sources-h">
    <div class="panel-head">
      <h2 id="sources-h">Sources</h2>
      <div class="tabs" role="tablist" aria-label="Sources">
        {#each enabled as s (s)}
          <button
            type="button"
            role="tab"
            id="source-tab-{s}"
            aria-selected={s === active}
            aria-controls="source-pane-{s}"
            class="tab"
            class:active={s === active}
            onclick={() => (chosen = s)}>{SOURCE_LABELS[s]}</button>
        {/each}
      </div>
    </div>
    {#if sources.errors.list}<p class="error" role="alert">Sources: {sources.errors.list}</p>{/if}
    <div role="tabpanel" id="source-pane-{active}" aria-labelledby="source-tab-{active}" data-source={active}>
      {#key active}
      <SourceTab
        source={active}
        view={sources.views[active] ?? null}
        loadError={sources.errors[active] ?? null}
        site={sources.summaries.find((s) => s.source === active)?.site ?? null}
        {operator}
        {now}
        onReload={() => sources.reload(active)}
        onView={(v) => sources.setView(active, v)}
      />
      {/key}
    </div>
  </section>
{/if}
