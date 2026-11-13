<script lang="ts">
  import { Dashboard } from './lib/dashboard.svelte';
  import type { MessageFilter } from './lib/messages';
  import { Poller } from './lib/poll.svelte';
  import { SOURCES_POLL_MS, SourcesState } from './lib/sources.svelte';
  import { effectiveTheme, saveTheme, type ThemeChoice } from './lib/theme';
  import Activity from './panels/Activity.svelte';
  import Connect from './panels/Connect.svelte';
  import Decisions from './panels/Decisions.svelte';
  import Header from './panels/Header.svelte';
  import Messages from './panels/Messages.svelte';
  import Sessions from './panels/Sessions.svelte';
  import Sources from './panels/Sources.svelte';
  import Tasks from './panels/Tasks.svelte';

  const dash = new Dashboard();
  const poller = new Poller(() => dash.refresh());
  const sources = new SourcesState();
  const sourcePoller = new Poller(() => sources.refresh(), SOURCES_POLL_MS);

  let now = $state(Date.now());
  let theme = $state<ThemeChoice>(effectiveTheme());
  let showConnect = $state(false);

  $effect(() => {
    poller.start();
    sourcePoller.start();
    const clock = setInterval(() => (now = Date.now()), 1000);
    return () => {
      poller.stop();
      sourcePoller.stop();
      clearInterval(clock);
    };
  });

  /** One pause button for both polls. */
  function togglePause(): void {
    if (poller.paused) {
      poller.resume();
      sourcePoller.resume();
    } else {
      poller.pause();
      sourcePoller.pause();
    }
  }

  function toggleTheme(): void {
    theme = theme === 'dark' ? 'light' : 'dark';
    saveTheme(theme);
  }

  function setFilter(f: MessageFilter): void {
    dash.filter = f;
    changed();
  }

  function changed(): void {
    // After an operator action, re-read state right away (forced: works while paused too).
    void poller.run(true);
  }
</script>

<div class="app">
  <Header
    overview={dash.overview}
    paused={poller.paused}
    dark={theme === 'dark'}
    error={dash.error}
    onTogglePause={togglePause}
    onToggleTheme={toggleTheme}
    onUnreadClick={() => setFilter({ mode: 'unread' })}
    onConnect={() => (showConnect = !showConnect)}
  />
  {#if showConnect}<Connect onClose={() => (showConnect = false)} />{/if}
  <main class="grid">
    <div class="area-sessions">
      <Sessions sessions={dash.overview?.sessions ?? []} tasks={dash.overview ? [...dash.overview.tasks.claimed, ...dash.overview.tasks.open] : []} {now} />
    </div>
    <div class="area-activity"><Activity items={dash.overview?.events ?? []} {now} /></div>
    <div class="area-messages">
      <Messages
        messages={dash.messages}
        filter={dash.filter}
        agents={dash.agents}
        operator={dash.operator}
        {now}
        onFilter={(f) => setFilter(f)}
        onChanged={changed}
      />
    </div>
    <div class="area-decisions"><Decisions decisions={dash.overview?.decisions ?? []} {now} /></div>
    <div class="area-tasks">
      {#if dash.overview}
        <Tasks tasks={dash.overview.tasks} operator={dash.operator} {now} onChanged={changed} />
      {/if}
    </div>
    {#if sources.enabled.length > 0}
      <div class="area-sources"><Sources {sources} operator={dash.operator} {now} /></div>
    {/if}
  </main>
</div>
