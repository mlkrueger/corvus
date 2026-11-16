<script lang="ts">
  import { appInfo, errorText, installCli, type AppInfo } from '../lib/api';

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  let info = $state<AppInfo | null>(null);
  let loadError = $state<string | null>(null);
  let installError = $state<string | null>(null);
  let installing = $state(false);

  async function load(): Promise<void> {
    try {
      info = await appInfo();
      loadError = null;
    } catch (e) {
      info = null;
      loadError = errorText(e);
    }
  }

  async function install(): Promise<void> {
    installing = true;
    installError = null;
    try {
      await installCli();
      await load();
    } catch (e) {
      installError = errorText(e);
    } finally {
      installing = false;
    }
  }

  $effect(() => {
    void load();
  });
</script>

<section class="panel connect" aria-labelledby="connect-h">
  <div class="panel-head">
    <h2 id="connect-h">Connect an agent</h2>
    <button type="button" onclick={onClose}>Close</button>
  </div>
  {#if info}
    <p class="muted">Corvus {info.version}</p>
    <p>
      Binary: <code>{info.corvus_path ?? 'not found'}</code>
    </p>
    <p>
      Command-line tool: {info.cli_installed ? 'installed on PATH' : 'not installed'}
      {#if !info.cli_installed && info.corvus_path}
        <button type="button" onclick={install} disabled={installing}>Install command-line tool</button>
      {/if}
    </p>
    {#if installError}<p class="error" role="alert">{installError}</p>{/if}
    <h3>Adapters</h3>
    <ul class="rows">
      {#each info.adapters as a (a.name)}
        <li class="row"><strong>{a.name}</strong> <span class="badge">{a.installed ? 'installed' : 'not installed'}</span></li>
      {/each}
    </ul>
    <h3>MCP configuration</h3>
    <pre class="snippet">{info.mcp_snippet}</pre>
    <h3>Hook configuration</h3>
    <p class="muted">
      For Claude Code: shows new mail and wakes an idle session. Run <code>corvus setup claude</code> to add this to
      <code>~/.claude/settings.json</code>, or paste it yourself. Allow the Corvus tools (<code>mcp__corvus</code>) so a
      woken session can answer.
    </p>
    <pre class="snippet">{info.hook_snippet}</pre>
  {:else if loadError}
    <p class="empty" role="status">
      Connection details are only available in the desktop app ({loadError}). You can still connect an agent by running
      <code>corvus mcp</code> as its MCP server.
    </p>
  {:else}
    <p class="empty">Loading...</p>
  {/if}
</section>
