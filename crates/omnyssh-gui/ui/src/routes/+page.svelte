<!-- Content follows the active entity (tech-gui.md §2): Dashboard, Snippets, Settings,
     or one (or two split) live terminal/SFTP session(s). All sessions stay mounted in the
     persistent layer so scrollback, SSH channels, and split toggles never lose state. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { listHosts } from '$lib/ipc/commands';
  import { hosts } from '$lib/stores/hosts';
  import { lastError } from '$lib/stores/notifications';
  import { activeEntity } from '$lib/stores/activeEntity';
  import { sessions } from '$lib/stores/sessions';
  import { splitLayout } from '$lib/stores/layout';
  import AppShell from '$lib/components/AppShell.svelte';
  import Dashboard from '$lib/screens/Dashboard.svelte';
  import Snippets from '$lib/screens/Snippets.svelte';
  import Settings from '$lib/screens/Settings.svelte';
  import TerminalView from '$lib/screens/TerminalView.svelte';
  import SftpView from '$lib/screens/SftpView.svelte';

  onMount(async () => {
    if (typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window)) return;
    try {
      hosts.set(await listHosts());
    } catch (err) {
      lastError.set(err instanceof Error ? err.message : String(err));
    }
  });

  const activeSessionId = $derived($activeEntity.kind === 'session' ? $activeEntity.id : null);
  const selectorActive = $derived(
    $activeEntity.kind === 'dashboard' ||
      $activeEntity.kind === 'snippets' ||
      $activeEntity.kind === 'settings'
  );

  // Split mode is active when the primary active session is a terminal and a distinct
  // secondary terminal session exists.
  const splitActive = $derived.by(() => {
    if (selectorActive || activeSessionId == null || $splitLayout.direction === 'none') return false;
    const secId = $splitLayout.secondarySessionId;
    if (secId == null || secId === activeSessionId) return false;
    const primary = $sessions.find((s) => s.id === activeSessionId);
    const secondary = $sessions.find((s) => s.id === secId);
    return primary?.kind === 'terminal' && secondary?.kind === 'terminal';
  });

  function terminalPaneClass(id: number): string {
    if (!splitActive) return 'inset-0';
    if ($splitLayout.direction === 'horizontal') {
      return id === activeSessionId
        ? 'inset-y-0 left-0 right-1/2 border-r border-default'
        : 'inset-y-0 left-1/2 right-0';
    }
    return id === activeSessionId
      ? 'inset-x-0 top-0 bottom-1/2 border-b border-default'
      : 'inset-x-0 top-1/2 bottom-0';
  }
</script>

<AppShell>
  <div class="relative h-full">
    {#each $sessions as s (s.id)}
      {#if s.kind === 'terminal'}
        {@const isPrimary = !selectorActive && activeSessionId === s.id}
        {@const isSecondary = splitActive && $splitLayout.secondarySessionId === s.id}
        <TerminalView
          session={s}
          active={isPrimary || isSecondary}
          paneClass={terminalPaneClass(s.id)}
          isSplit={splitActive && (isPrimary || isSecondary)}
          focused={!splitActive || (isPrimary ? $splitLayout.focusedPane === 'primary' : $splitLayout.focusedPane === 'secondary')}
          onPaneFocus={() => splitActive && splitLayout.focus(isPrimary ? 'primary' : 'secondary')}
        />
      {:else}
        <SftpView session={s} active={!selectorActive && activeSessionId === s.id} />
      {/if}
    {/each}

    {#if selectorActive}
      <!-- Inset the scroll container (not the content) below the macOS title-bar strip,
           so selector content scrolls within its pane and never under the traffic lights. -->
      <div class="absolute inset-0 pt-[var(--titlebar-h)]">
        <div class="h-full overflow-auto overscroll-contain">
          {#if $activeEntity.kind === 'dashboard'}
            <Dashboard />
          {:else if $activeEntity.kind === 'snippets'}
            <Snippets />
          {:else if $activeEntity.kind === 'settings'}
            <Settings />
          {/if}
        </div>
      </div>
    {/if}
  </div>
</AppShell>
