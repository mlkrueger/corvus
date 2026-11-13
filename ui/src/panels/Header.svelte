<script lang="ts">
  import type { Overview } from '../lib/types/Overview';

  interface Props {
    overview: Overview | null;
    paused: boolean;
    dark: boolean;
    error: string | null;
    onTogglePause: () => void;
    onToggleTheme: () => void;
    onUnreadClick: () => void;
    onConnect: () => void;
  }
  let { overview, paused, dark, error, onTogglePause, onToggleTheme, onUnreadClick, onConnect }: Props = $props();

  const PRESENCE = ['working', 'active', 'idle', 'offline'] as const;
</script>

<header class="topbar">
  <h1>Corvus</h1>
  <div class="chips" aria-label="Summary">
    {#if overview}
      {#each PRESENCE as p (p)}
        <span class="chip presence-{p}" data-chip="presence-{p}">{p} <b>{overview.presence_counts[p] ?? 0}</b></span>
      {/each}
      <span class="chip" data-chip="open">open tasks <b>{overview.stats.open_tasks}</b></span>
      <span class="chip" data-chip="claimed">claimed <b>{overview.stats.claimed_tasks}</b></span>
      <button type="button" class="chip chip-button" data-chip="unread" onclick={onUnreadClick} title="Show unread messages">
        unread <b>{overview.stats.unread_total}</b>
      </button>
      <span class="chip" class:warn={overview.stats.expired_leases > 0} data-chip="expired">
        expired leases <b>{overview.stats.expired_leases}</b>
      </span>
      <span class="chip" class:warn={overview.blocked_by_cancelled > 0} data-chip="blocked-cancelled">
        blocked by cancelled <b>{overview.blocked_by_cancelled}</b>
      </span>
    {:else}
      <span class="chip">loading...</span>
    {/if}
  </div>
  <div class="controls">
    {#if error}<span class="chip warn" role="alert" data-testid="poll-error">{error}</span>{/if}
    <button type="button" onclick={onConnect}>Connect an agent</button>
    <button type="button" onclick={onTogglePause} aria-pressed={paused}>{paused ? 'Resume' : 'Pause'}</button>
    <button type="button" onclick={onToggleTheme} aria-label="Toggle theme">{dark ? 'Light theme' : 'Dark theme'}</button>
  </div>
</header>
