<script lang="ts">
  import { Icon } from '$lib/theme';
  import { vaultState } from '$lib/stores/vault';
  import { vaultUnlock, reloadHosts } from '$lib/ipc/commands';
  import { t } from '$lib/i18n';

  let password = $state('');
  let unlocking = $state(false);
  let error = $state<string | null>(null);

  async function submit(): Promise<void> {
    if (!password || unlocking) return;
    unlocking = true;
    error = null;
    try {
      const status = await vaultUnlock(password);
      vaultState.set(status);
      password = '';
      await reloadHosts();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      unlocking = false;
    }
  }
</script>

{#if $vaultState.mode === 'masterPassword' && !$vaultState.unlocked}
  <div
    role="region"
    aria-label={$t('settings.vault_section')}
    class="flex flex-wrap items-center justify-between gap-3 border-b border-default bg-surface-raised px-4 py-2 text-xs"
  >
    <div class="flex items-center gap-2 text-status-warn">
      <Icon name="lock" size={14} />
      <span>{$t('settings.vault_locked_banner')}</span>
    </div>
    <form
      class="flex items-center gap-2"
      onsubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <input
        bind:value={password}
        type="password"
        placeholder={$t('settings.vault_current_pw')}
        class="w-48 rounded-lg border border-default bg-surface-inset px-2.5 py-1 text-xs text-fg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus"
      />
      <button
        type="submit"
        disabled={!password || unlocking}
        class="rounded-lg bg-accent px-3 py-1 text-xs font-medium text-accent-fg disabled:opacity-50"
      >
        {$t('settings.vault_unlock_now')}
      </button>
      {#if error}
        <span class="text-status-crit">{error}</span>
      {/if}
    </form>
  </div>
{/if}
