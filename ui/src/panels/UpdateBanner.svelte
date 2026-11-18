<script lang="ts">
  import type { UpdateState } from '../lib/update.svelte';

  interface Props {
    updates: UpdateState;
  }
  let { updates }: Props = $props();
</script>

{#if updates.status?.available}
  <section class="panel update-banner" role="status" aria-label="Update available">
    <span>
      Corvus <strong>{updates.status.available}</strong> is available (this is {updates.status.current}).
    </span>
    <button type="button" onclick={() => updates.install()} disabled={updates.installing}>
      {updates.installing ? 'Installing...' : 'Install and restart'}
    </button>
    {#if updates.error}<span class="error" role="alert">{updates.error}</span>{/if}
  </section>
{/if}
