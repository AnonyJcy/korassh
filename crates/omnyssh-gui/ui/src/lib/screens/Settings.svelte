<script lang="ts">
  // Minimal settings (tech-gui.md §4.3): the light/dark theme mirrored from the sidebar
  // (§5.1), the metric auto-refresh interval and the tray (tauri-plugin-store UI prefs),
  // and the update preferences (`check_on_startup`) + a manual update check. Update prefs
  // persist to the shared config via `save_update_config`; the rest are frontend prefs.
  import { onMount } from 'svelte';
  import type { SyncConfigDto, SyncProviderDto, UpdateConfigDto } from '$lib/bindings';
  import { Surface, Icon } from '$lib/theme';
  import Select from '$lib/components/Select.svelte';
  import { theme } from '$lib/stores/theme';
  import {
    fontPrefs,
    harmonyFontState,
    retryHarmonyFontLoad,
    UI_FONT_PRESETS,
    TERMINAL_FONT_PRESETS,
    DEFAULT_FONT_PREFS
  } from '$lib/stores/font';
  import {
    terminalPrefs,
    TERMINAL_COLOR_SCHEMES,
    CURSOR_STYLES,
    SCROLLBACK_OPTIONS,
    HIGHLIGHT_COLORS,
    MIN_FONT_SIZE,
    MAX_FONT_SIZE,
    type HighlightColor
  } from '$lib/stores/terminalPrefs';
  import { vaultState, refreshVaultStatus } from '$lib/stores/vault';
  import { streamerMode } from '$lib/stores/streamer';
  import { refreshInterval, REFRESH_OPTIONS } from '$lib/stores/settings';
  import { traySupport, trayBehavior } from '$lib/stores/tray';
  import { clipboardPrefs, type RightClick } from '$lib/stores/clipboard';
  import { confirmQuit } from '$lib/stores/quit';
  import { isMac } from '$lib/platform';
  import { offerUpdate } from '$lib/stores/update';
  import { lastError } from '$lib/stores/notifications';
  import {
    checkUpdate,
    loadUpdateConfig,
    saveUpdateConfig,
    reloadHosts,
    listSnippets,
    vaultUnlock,
    vaultLock,
    vaultSetMasterPassword,
    vaultRemoveMasterPassword,
    exportBackup,
    importBackup,
    loadSyncConfig,
    saveSyncConfig,
    syncPush,
    syncPull,
    saveTextFile
  } from '$lib/ipc/commands';
  import { snippets } from '$lib/stores/snippets';
  import { locale, SUPPORTED_LOCALES, t } from '$lib/i18n';
  import { installedVersion } from '$lib/ipc/appInfo';

  const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));
  const formatInterval = (secs: number): string => (secs < 60 ? `${secs}s` : `${secs / 60}m`);

  let updateConfig = $state<UpdateConfigDto | null>(null);
  let version = $state<string | null>(null);
  let newRuleLabel = $state('');
  let newRulePattern = $state('');
  let newRuleColor = $state<HighlightColor>('yellow');

  // Vault UI state
  let vaultAction = $state<'none' | 'set' | 'remove' | 'unlock'>('none');
  let vaultCurrentPw = $state('');
  let vaultNewPw = $state('');
  let vaultConfirmPw = $state('');
  let vaultBusy = $state(false);
  let vaultMsg = $state<{ ok: boolean; text: string } | null>(null);

  // Backup & Sync UI state
  let syncPassphrase = $state('');
  let syncBusy = $state(false);
  let syncMsg = $state<{ ok: boolean; text: string } | null>(null);
  let importFileInput = $state<HTMLInputElement>();
  let importReplace = $state(false);
  let syncCfg = $state<SyncConfigDto>({
    provider: 'none',
    webdavUrl: '',
    webdavUsername: '',
    hasWebdavAuth: false,
    webdavRemotePath: 'korassh-sync.korabak',
    gitRepoDir: '',
    gitRemote: 'origin',
    gitBranch: 'main',
    gitFileName: 'korassh-sync.korabak'
  });
  let syncWebdavPassword = $state('');

  type CheckState =
    | { kind: 'idle' }
    | { kind: 'checking' }
    | { kind: 'upToDate' }
    | { kind: 'available'; version: string }
    | { kind: 'error'; message: string };
  let check = $state<CheckState>({ kind: 'idle' });

  onMount(async () => {
    // Not awaited, so the update prefs never wait on it; a failed read leaves the row out.
    installedVersion().then((v) => (version = v), () => {});
    void refreshVaultStatus();
    if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
      loadSyncConfig()
        .then((c) => {
          if (c && typeof c === 'object' && 'provider' in c) syncCfg = c;
        })
        .catch(() => {});
    }
    try {
      updateConfig = await loadUpdateConfig();
    } catch (e) {
      lastError.set(message(e));
    }
  });

  function addHighlightRule(): void {
    const pattern = newRulePattern.trim();
    if (!pattern) return;
    const label = newRuleLabel.trim() || pattern.slice(0, 16);
    terminalPrefs.update({
      highlightRules: [
        ...$terminalPrefs.highlightRules,
        { id: `custom-${Date.now()}`, label, pattern, color: newRuleColor, enabled: true }
      ]
    });
    newRuleLabel = '';
    newRulePattern = '';
  }

  async function submitVaultAction(): Promise<void> {
    if (vaultBusy) return;
    vaultMsg = null;
    if (vaultAction === 'set' && vaultNewPw !== vaultConfirmPw) {
      vaultMsg = { ok: false, text: $t('settings.vault_pw_mismatch') };
      return;
    }
    vaultBusy = true;
    try {
      let next;
      if (vaultAction === 'unlock') {
        next = await vaultUnlock(vaultCurrentPw);
        await reloadHosts();
      } else if (vaultAction === 'set') {
        next = await vaultSetMasterPassword(
          $vaultState.mode === 'masterPassword' ? vaultCurrentPw : null,
          vaultNewPw
        );
      } else if (vaultAction === 'remove') {
        next = await vaultRemoveMasterPassword(vaultCurrentPw);
      }
      if (next) vaultState.set(next);
      vaultAction = 'none';
      vaultCurrentPw = '';
      vaultNewPw = '';
      vaultConfirmPw = '';
      vaultMsg = { ok: true, text: $t('settings.vault_updated') };
    } catch (e) {
      vaultMsg = { ok: false, text: message(e) };
    } finally {
      vaultBusy = false;
    }
  }

  async function lockVaultNow(): Promise<void> {
    try {
      const next = await vaultLock();
      vaultState.set(next);
      await reloadHosts();
    } catch (e) {
      vaultMsg = { ok: false, text: message(e) };
    }
  }

  async function handleExportBackup(): Promise<void> {
    if (!syncPassphrase.trim() || syncBusy) return;
    syncBusy = true;
    syncMsg = null;
    try {
      const bundle = await exportBackup(syncPassphrase);
      const stamp = new Date().toISOString().slice(0, 10);
      const savedPath = await saveTextFile(`korassh-backup-${stamp}.korabak`, bundle);
      syncMsg = { ok: true, text: $t('settings.backup_exported', { path: savedPath }) };
    } catch (e) {
      syncMsg = { ok: false, text: message(e) };
    } finally {
      syncBusy = false;
    }
  }

  function triggerImport(replace: boolean): void {
    importReplace = replace;
    importFileInput?.click();
  }

  async function handleImportFile(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file || !syncPassphrase.trim() || syncBusy) return;
    syncBusy = true;
    syncMsg = null;
    try {
      const text = await file.text();
      const summary = await importBackup(text, syncPassphrase, importReplace);
      await reloadHosts();
      snippets.set(await listSnippets().catch(() => []));
      syncMsg = {
        ok: true,
        text: $t('settings.backup_imported', {
          hosts: summary.hostsImported,
          snippets: summary.snippetsImported
        })
      };
    } catch (err) {
      syncMsg = { ok: false, text: message(err) };
    } finally {
      syncBusy = false;
    }
  }

  async function handleSaveSync(): Promise<void> {
    if (syncBusy) return;
    syncBusy = true;
    syncMsg = null;
    try {
      syncCfg = await saveSyncConfig({
        provider: syncCfg.provider,
        webdavUrl: syncCfg.webdavUrl,
        webdavUsername: syncCfg.webdavUsername,
        webdavPassword: syncWebdavPassword || undefined,
        webdavRemotePath: syncCfg.webdavRemotePath,
        gitRepoDir: syncCfg.gitRepoDir,
        gitRemote: syncCfg.gitRemote,
        gitBranch: syncCfg.gitBranch,
        gitFileName: syncCfg.gitFileName
      });
      syncWebdavPassword = '';
      syncMsg = { ok: true, text: $t('settings.sync_saved') };
    } catch (e) {
      syncMsg = { ok: false, text: message(e) };
    } finally {
      syncBusy = false;
    }
  }

  async function handleSyncPush(): Promise<void> {
    if (!syncPassphrase.trim() || syncBusy) return;
    syncBusy = true;
    syncMsg = null;
    try {
      const msg = await syncPush(syncPassphrase);
      syncMsg = { ok: true, text: msg };
    } catch (e) {
      syncMsg = { ok: false, text: message(e) };
    } finally {
      syncBusy = false;
    }
  }

  async function handleSyncPull(): Promise<void> {
    if (!syncPassphrase.trim() || syncBusy) return;
    syncBusy = true;
    syncMsg = null;
    try {
      const summary = await syncPull(syncPassphrase, false);
      await reloadHosts();
      snippets.set(await listSnippets().catch(() => []));
      syncMsg = {
        ok: true,
        text: $t('settings.backup_imported', {
          hosts: summary.hostsImported,
          snippets: summary.snippetsImported
        })
      };
    } catch (e) {
      syncMsg = { ok: false, text: message(e) };
    } finally {
      syncBusy = false;
    }
  }

  // Persist immediately; revert the optimistic flip if the write fails. Read-modify-write
  // fresh rather than from the mount-time cache: the update banner may have written a
  // `skipVersion` out-of-band, and `save_update_config` replaces the whole [update]
  // section — writing a stale copy would clobber that skip.
  async function toggleCheckOnStartup(): Promise<void> {
    if (!updateConfig) return;
    const previous = updateConfig;
    const desired = !previous.checkOnStartup;
    updateConfig = { ...previous, checkOnStartup: desired };
    try {
      const current = await loadUpdateConfig();
      const next = { ...current, checkOnStartup: desired };
      await saveUpdateConfig(next);
      updateConfig = next;
    } catch (e) {
      updateConfig = previous;
      lastError.set(message(e));
    }
  }

  async function checkNow(): Promise<void> {
    check = { kind: 'checking' };
    try {
      const info = await checkUpdate();
      if (info) {
        offerUpdate(info); // raise the banner too
        check = { kind: 'available', version: info.version };
      } else {
        check = { kind: 'upToDate' };
      }
    } catch (e) {
      check = { kind: 'error', message: message(e) };
    }
  }

  const RIGHT_CLICK_OPTIONS: RightClick[] = ['paste', 'copyOrPaste', 'menu', 'off'];
  const copyKeys = isMac ? 'Cmd+C, Ctrl+Insert' : 'Ctrl+Shift+C, Ctrl+Insert';
  const pasteKeys = isMac ? 'Cmd+V, Shift+Insert' : 'Ctrl+Shift+V, Shift+Insert';
  const copyAllKey = isMac ? '⇧⌘A' : 'Ctrl+Shift+A';

  const seg =
    'rounded-lg px-3 py-1.5 text-sm whitespace-nowrap transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
  const segFull =
    'flex-auto rounded-lg px-2.5 py-1.5 text-center text-sm whitespace-nowrap transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
  const segState = (active: boolean): string =>
    active ? 'bg-accent text-accent-fg' : 'text-muted hover:bg-surface-inset hover:text-fg';
</script>

<section class="mx-auto min-h-full max-w-2xl p-6">
  <h1 class="mb-5 text-lg font-semibold tracking-tight">{$t('settings.title')}</h1>

  <div class="space-y-4">
    <!-- Language / 语言 -->
    <Surface class="p-5">
      <h2 class="mb-3 text-sm font-semibold">{$t('settings.language')}</h2>
      <div class="flex items-center justify-between gap-4">
        <div>
          <p class="text-sm">{$t('settings.language')}</p>
          <p class="text-xs text-muted">{$t('settings.language_desc')}</p>
        </div>
        <div class="flex gap-1 rounded-xl bg-surface-inset p-1">
          {#each SUPPORTED_LOCALES as loc (loc.code)}
            <button
              type="button"
              class="{seg} {segState($locale === loc.code)}"
              aria-pressed={$locale === loc.code}
              onclick={() => locale.set(loc.code)}
            >
              {loc.label}
            </button>
          {/each}
        </div>
      </div>
    </Surface>

    <!-- Appearance -->
    <Surface class="p-5">
      <div class="mb-3 flex items-center justify-between gap-4">
        <h2 class="text-sm font-semibold">{$t('settings.appearance')}</h2>
        {#if $fontPrefs.ui !== DEFAULT_FONT_PREFS.ui || $fontPrefs.terminal !== DEFAULT_FONT_PREFS.terminal || $fontPrefs.uiCustom !== '' || $fontPrefs.terminalCustom !== ''}
          <button
            type="button"
            class="text-xs text-muted transition hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
            onclick={() => fontPrefs.reset()}
          >
            {$t('settings.font_reset')}
          </button>
        {/if}
      </div>
      <div class="space-y-4">
        <div class="flex items-center justify-between gap-4">
          <div>
            <p class="text-sm">{$t('settings.theme')}</p>
            <p class="text-xs text-muted">{$t('settings.theme_desc')}</p>
          </div>
          <div class="flex gap-1 rounded-xl bg-surface-inset p-1">
            <button
              type="button"
              class="{seg} {segState($theme === 'light')}"
              aria-pressed={$theme === 'light'}
              onclick={() => theme.set('light')}
            >
              <span class="flex items-center gap-1.5"><Icon name="sun" size={14} /> {$t('settings.theme_light')}</span>
            </button>
            <button
              type="button"
              class="{seg} {segState($theme === 'dark')}"
              aria-pressed={$theme === 'dark'}
              onclick={() => theme.set('dark')}
            >
              <span class="flex items-center gap-1.5"><Icon name="moon" size={14} /> {$t('settings.theme_dark')}</span>
            </button>
          </div>
        </div>

        <div class="space-y-2.5 border-t border-default pt-4">
          <div>
            <p class="text-sm">{$t('settings.ui_font')}</p>
            <p class="text-xs text-muted">{$t('settings.ui_font_desc')}</p>
          </div>
          <div data-font-group="ui" class="flex w-full items-center gap-1 rounded-xl bg-surface-inset p-1">
            {#each UI_FONT_PRESETS as preset (preset)}
              <button
                type="button"
                class="{segFull} {segState($fontPrefs.ui === preset)}"
                aria-pressed={$fontPrefs.ui === preset}
                onclick={() => fontPrefs.update({ ui: preset })}
              >
                {$t(`settings.ui_font_${preset}`)}
              </button>
            {/each}
          </div>
          {#if $fontPrefs.ui === 'harmonyos' && $harmonyFontState !== 'idle'}
            <div class="flex items-center justify-between gap-2 text-xs text-muted">
              <span class={$harmonyFontState === 'offline' ? 'text-status-warn' : ''}>
                {$t(`settings.harmony_status_${$harmonyFontState}`)}
              </span>
              {#if $harmonyFontState === 'offline'}
                <button
                  type="button"
                  class="text-xs text-fg underline underline-offset-2 transition hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
                  onclick={() => retryHarmonyFontLoad()}
                >
                  {$t('settings.harmony_retry')}
                </button>
              {/if}
            </div>
          {/if}
          {#if $fontPrefs.ui === 'custom'}
            <input
              type="text"
              value={$fontPrefs.uiCustom}
              oninput={(e) => fontPrefs.update({ uiCustom: e.currentTarget.value })}
              placeholder={$t('settings.ui_font_custom_placeholder')}
              aria-label={$t('settings.ui_font')}
              class="w-full rounded-xl border border-default bg-surface-inset px-3 py-2 text-sm text-fg placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
            />
          {/if}
          <p class="rounded-lg border border-default bg-surface-inset px-3 py-2 text-xs text-fg">
            {$t('settings.ui_font_preview')}
          </p>
        </div>

        <div class="space-y-2.5 border-t border-default pt-4">
          <div>
            <p class="text-sm">{$t('settings.terminal_font')}</p>
            <p class="text-xs text-muted">{$t('settings.terminal_font_desc')}</p>
          </div>
          <div data-font-group="terminal" class="flex w-full items-center gap-1 rounded-xl bg-surface-inset p-1">
            {#each TERMINAL_FONT_PRESETS as preset (preset)}
              <button
                type="button"
                class="{segFull} {segState($fontPrefs.terminal === preset)}"
                aria-pressed={$fontPrefs.terminal === preset}
                onclick={() => fontPrefs.update({ terminal: preset })}
              >
                {$t(`settings.terminal_font_${preset}`)}
              </button>
            {/each}
          </div>
          {#if $fontPrefs.terminal === 'custom'}
            <input
              type="text"
              value={$fontPrefs.terminalCustom}
              oninput={(e) => fontPrefs.update({ terminalCustom: e.currentTarget.value })}
              placeholder={$t('settings.terminal_font_custom_placeholder')}
              aria-label={$t('settings.terminal_font')}
              class="w-full rounded-xl border border-default bg-surface-inset px-3 py-2 font-mono text-sm text-fg placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
            />
          {/if}
        </div>
      </div>
    </Surface>

    <!-- Privacy -->
    <Surface class="p-5">
      <h2 class="mb-3 text-sm font-semibold">{$t('settings.streamer_mode')}</h2>
      <div class="flex items-center justify-between gap-4">
        <div class="min-w-0">
          <p class="text-sm">{$t('settings.streamer_mode')}</p>
          <p class="text-xs text-muted">
            {$t('settings.streamer_mode_desc')}
          </p>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={$streamerMode}
          aria-label={$t('settings.streamer_mode')}
          onclick={() => streamerMode.toggle()}
          class="relative h-6 w-11 shrink-0 rounded-full transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus {$streamerMode
            ? 'bg-accent'
            : 'bg-surface-inset'}"
        >
          <span
            class="absolute top-0.5 h-5 w-5 rounded-full bg-surface shadow-soft transition-[left] {$streamerMode
              ? 'left-[1.375rem]'
              : 'left-0.5'}"
          ></span>
        </button>
      </div>
    </Surface>

    <!-- Terminal copy & paste: PuTTY-style by default (stores/clipboard). -->
    <Surface class="p-5">
      <div class="mb-3 flex items-center justify-between gap-4">
        <h2 class="text-sm font-semibold">{$t('settings.clipboard')}</h2>
        <button
          type="button"
          class="text-xs text-muted transition hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
          onclick={() => clipboardPrefs.reset()}
        >
          {$t('settings.clipboard_reset')}
        </button>
      </div>
      <div class="space-y-4">
        {@render traySwitch(
          $t('settings.copy_on_select'),
          $t('settings.copy_on_select_desc'),
          $clipboardPrefs.copyOnSelect,
          true,
          () => clipboardPrefs.update({ copyOnSelect: !$clipboardPrefs.copyOnSelect })
        )}

        <div class="space-y-2.5 border-t border-default pt-4">
          <div>
            <p class="text-sm">{$t('settings.right_click')}</p>
            <p class="text-xs text-muted">{$t(`settings.right_click_${$clipboardPrefs.rightClick}_desc`)}</p>
          </div>
          <div data-clipboard-group="rightClick" class="flex w-full items-center gap-1 rounded-xl bg-surface-inset p-1">
            {#each RIGHT_CLICK_OPTIONS as option (option)}
              <button
                type="button"
                class="{segFull} {segState($clipboardPrefs.rightClick === option)}"
                aria-pressed={$clipboardPrefs.rightClick === option}
                onclick={() => clipboardPrefs.update({ rightClick: option })}
              >
                {$t(`settings.right_click_${option}`)}
              </button>
            {/each}
          </div>
        </div>

        <div class="border-t border-default pt-4">
          {@render traySwitch(
            $t('settings.middle_click_paste'),
            $t('settings.middle_click_paste_desc'),
            $clipboardPrefs.middleClickPaste,
            true,
            () => clipboardPrefs.update({ middleClickPaste: !$clipboardPrefs.middleClickPaste })
          )}
        </div>

        {@render traySwitch(
          $t('settings.trim_trailing_spaces'),
          $t('settings.trim_trailing_spaces_desc'),
          $clipboardPrefs.trimTrailingWhitespace,
          true,
          () =>
            clipboardPrefs.update({ trimTrailingWhitespace: !$clipboardPrefs.trimTrailingWhitespace })
        )}

        {@render traySwitch(
          $t('settings.confirm_multi_line_paste'),
          $t('settings.confirm_multi_line_paste_desc'),
          $clipboardPrefs.confirmMultiLinePaste,
          true,
          () =>
            clipboardPrefs.update({ confirmMultiLinePaste: !$clipboardPrefs.confirmMultiLinePaste })
        )}

        <p class="border-t border-default pt-4 text-xs text-muted">
          {$t('settings.clipboard_shortcuts_hint', { copy: copyKeys, paste: pasteKeys, copyAll: copyAllKey })}
        </p>
      </div>
    </Surface>

    <!-- Dashboard -->
    <Surface class="p-5">
      <h2 class="mb-3 text-sm font-semibold">{$t('dashboard.title')}</h2>
      <div class="flex items-center justify-between gap-4">
        <div>
          <p class="text-sm">{$t('settings.metrics_refresh')}</p>
          <p class="text-xs text-muted">{$t('settings.metrics_refresh_desc')}</p>
        </div>
        <div class="flex shrink-0 gap-1 rounded-xl bg-surface-inset p-1">
          {#each REFRESH_OPTIONS as secs (secs)}
            <button
              type="button"
              class="{seg} tabular-nums {segState($refreshInterval === secs)}"
              aria-pressed={$refreshInterval === secs}
              onclick={() => refreshInterval.set(secs)}
            >
              {formatInterval(secs)}
            </button>
          {/each}
        </div>
      </div>
    </Surface>

    <!-- Window: the tray keeps sessions and tunnels up with no window on screen. macOS
         has no minimize event to act on, and keeps minimized windows in the Dock. -->
    <Surface class="p-5">
      <h2 class="mb-3 text-sm font-semibold">{$t('settings.window')}</h2>
      <div class="space-y-4">
        {#if !isMac}
          {@render traySwitch(
            $t('settings.minimize_to_tray'),
            $traySupport.available && !$traySupport.minimize
              ? $t('settings.minimize_to_tray_wayland')
              : $t('settings.minimize_to_tray_desc'),
            $trayBehavior.minimizeToTray && $traySupport.minimize,
            $traySupport.minimize,
            () => trayBehavior.update({ minimizeToTray: !$trayBehavior.minimizeToTray })
          )}
        {/if}
        {@render traySwitch(
          isMac ? $t('settings.close_to_menu_bar') : $t('settings.close_to_tray'),
          $t('settings.close_to_tray_desc'),
          $trayBehavior.closeToTray && $traySupport.available,
          $traySupport.available,
          () => trayBehavior.update({ closeToTray: !$trayBehavior.closeToTray })
        )}
        {#if !$traySupport.available}
          <p class="text-xs text-status-warn">
            {$t('settings.no_tray_warning')}
          </p>
        {:else if $trayBehavior.minimizeToTray || $trayBehavior.closeToTray}
          <!-- Some desktops keep tray icons out of sight (GNOME without the AppIndicator
               extension, an overflow menu); a second launch always finds the window. -->
          <p class="text-xs text-muted">
            {$t('settings.relaunch_hint')}
          </p>
        {/if}
        {@render traySwitch(
          $t('settings.confirm_quit'),
          $t('settings.confirm_quit_desc'),
          $confirmQuit,
          true,
          () => confirmQuit.toggle()
        )}
      </div>
    </Surface>

    <!-- Terminal customization (color schemes, zoom, line height, scrollback, cursor, highlights) -->
    <Surface class="p-5">
      <div class="mb-3 flex items-center justify-between gap-4">
        <h2 class="text-sm font-semibold">{$t('settings.terminal_section')}</h2>
        <button
          type="button"
          class="text-xs text-muted transition hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
          onclick={() => terminalPrefs.reset()}
        >
          {$t('settings.terminal_reset')}
        </button>
      </div>
      <div class="space-y-4">
        <div class="space-y-2">
          <div>
            <p class="text-sm">{$t('settings.terminal_scheme')}</p>
            <p class="text-xs text-muted">{$t('settings.terminal_scheme_desc')}</p>
          </div>
          <div class="grid grid-cols-4 gap-1 rounded-xl bg-surface-inset p-1">
            {#each TERMINAL_COLOR_SCHEMES as scheme (scheme)}
              <button
                type="button"
                class="{segFull} {segState($terminalPrefs.colorScheme === scheme)}"
                aria-pressed={$terminalPrefs.colorScheme === scheme}
                onclick={() => terminalPrefs.update({ colorScheme: scheme })}
              >
                {$t(`settings.scheme_${scheme}`)}
              </button>
            {/each}
          </div>
        </div>

        <div class="grid grid-cols-2 gap-4 border-t border-default pt-4">
          <div>
            <div class="mb-1 flex items-center justify-between text-xs">
              <span class="text-sm text-fg">{$t('settings.terminal_font_size')}</span>
              <span class="font-mono tabular-nums text-muted">{$terminalPrefs.fontSize}px</span>
            </div>
            <input
              type="range"
              min={MIN_FONT_SIZE}
              max={MAX_FONT_SIZE}
              step="1"
              value={$terminalPrefs.fontSize}
              oninput={(e) => terminalPrefs.update({ fontSize: Number(e.currentTarget.value) })}
              class="w-full accent-[var(--accent)]"
            />
          </div>
          <div>
            <div class="mb-1 flex items-center justify-between text-xs">
              <span class="text-sm text-fg">{$t('settings.terminal_line_height')}</span>
              <span class="font-mono tabular-nums text-muted">{$terminalPrefs.lineHeight.toFixed(2)}</span>
            </div>
            <input
              type="range"
              min="1.0"
              max="1.8"
              step="0.05"
              value={$terminalPrefs.lineHeight}
              oninput={(e) => terminalPrefs.update({ lineHeight: Number(e.currentTarget.value) })}
              class="w-full accent-[var(--accent)]"
            />
          </div>
        </div>

        <div class="space-y-2 border-t border-default pt-4">
          <p class="text-sm">{$t('settings.terminal_scrollback')}</p>
          <div class="flex w-full items-center gap-1 rounded-xl bg-surface-inset p-1">
            {#each SCROLLBACK_OPTIONS as lines (lines)}
              <button
                type="button"
                class="{segFull} tabular-nums {segState($terminalPrefs.scrollback === lines)}"
                aria-pressed={$terminalPrefs.scrollback === lines}
                onclick={() => terminalPrefs.update({ scrollback: lines })}
              >
                {lines.toLocaleString()}
              </button>
            {/each}
          </div>
        </div>

        <div class="flex items-center justify-between gap-4 border-t border-default pt-4">
          <p class="text-sm">{$t('settings.terminal_cursor_style')}</p>
          <div class="flex shrink-0 gap-1 rounded-xl bg-surface-inset p-1">
            {#each CURSOR_STYLES as style (style)}
              <button
                type="button"
                class="{seg} {segState($terminalPrefs.cursorStyle === style)}"
                aria-pressed={$terminalPrefs.cursorStyle === style}
                onclick={() => terminalPrefs.update({ cursorStyle: style })}
              >
                {$t(`settings.cursor_${style}`)}
              </button>
            {/each}
          </div>
        </div>

        {@render traySwitch(
          $t('settings.terminal_cursor_blink'),
          '',
          $terminalPrefs.cursorBlink,
          true,
          () => terminalPrefs.update({ cursorBlink: !$terminalPrefs.cursorBlink })
        )}

        <div class="space-y-2.5 border-t border-default pt-4">
          {@render traySwitch(
            $t('settings.terminal_highlights'),
            $t('settings.terminal_highlights_desc'),
            $terminalPrefs.highlightEnabled,
            true,
            () => terminalPrefs.update({ highlightEnabled: !$terminalPrefs.highlightEnabled })
          )}
          {#if $terminalPrefs.highlightEnabled}
            <div class="space-y-1.5">
              {#each $terminalPrefs.highlightRules as rule (rule.id)}
                <div class="flex items-center gap-2 rounded-lg bg-surface-inset px-2.5 py-1.5 text-xs">
                  <input
                    type="checkbox"
                    checked={rule.enabled}
                    onchange={() =>
                      terminalPrefs.update({
                        highlightRules: $terminalPrefs.highlightRules.map((r) =>
                          r.id === rule.id ? { ...r, enabled: !r.enabled } : r
                        )
                      })}
                  />
                  <span class="w-16 shrink-0 truncate font-medium text-fg">{rule.label}</span>
                  <span class="min-w-0 flex-1 truncate font-mono text-muted">{rule.pattern}</span>
                  <span class="shrink-0 rounded px-1.5 py-0.5 text-[10px] uppercase text-faint">{rule.color}</span>
                  <button
                    type="button"
                    class="text-muted hover:text-status-crit"
                    onclick={() =>
                      terminalPrefs.update({
                        highlightRules: $terminalPrefs.highlightRules.filter((r) => r.id !== rule.id)
                      })}
                  >
                    <Icon name="close" size={12} />
                  </button>
                </div>
              {/each}
              <div class="flex items-center gap-1.5 pt-1">
                <input
                  bind:value={newRuleLabel}
                  type="text"
                  placeholder={$t('settings.highlight_label_placeholder')}
                  class="w-24 rounded-lg border border-default bg-surface-inset px-2 py-1 text-xs text-fg"
                />
                <input
                  bind:value={newRulePattern}
                  type="text"
                  placeholder={$t('settings.highlight_pattern_placeholder')}
                  class="min-w-0 flex-1 rounded-lg border border-default bg-surface-inset px-2 py-1 font-mono text-xs text-fg"
                />
                <Select
                  bind:value={newRuleColor}
                  class="rounded-lg border border-default bg-surface-inset px-2 py-1 text-xs text-fg"
                >
                  {#each HIGHLIGHT_COLORS as col (col)}
                    <option value={col}>{col}</option>
                  {/each}
                </Select>
                <button
                  type="button"
                  class="rounded-lg bg-accent px-2.5 py-1 text-xs font-medium text-accent-fg"
                  onclick={addHighlightRule}
                >
                  {$t('settings.highlight_add')}
                </button>
              </div>
            </div>
          {/if}
        </div>
      </div>
    </Surface>

    <!-- Credential Vault -->
    <Surface class="p-5">
      <div class="mb-2 flex items-center justify-between gap-4">
        <h2 class="text-sm font-semibold">{$t('settings.vault_section')}</h2>
        <span class="rounded-full bg-surface-inset px-2.5 py-0.5 text-xs {$vaultState.unlocked ? 'text-status-ok' : 'text-status-warn'}">
          {#if $vaultState.mode === 'auto'}
            {$t('settings.vault_status_auto')}
          {:else if $vaultState.unlocked}
            {$t('settings.vault_status_unlocked')}
          {:else}
            {$t('settings.vault_status_locked')}
          {/if}
        </span>
      </div>
      <p class="text-xs text-muted">
        {$vaultState.mode === 'auto' ? $t('settings.vault_auto_desc') : $t('settings.vault_master_desc')}
      </p>
      <div class="mt-3 flex flex-wrap gap-2">
        {#if $vaultState.mode === 'auto'}
          <button
            type="button"
            class="rounded-full border border-strong px-3.5 py-1.5 text-xs text-fg transition hover:bg-accent hover:text-accent-fg"
            onclick={() => { vaultAction = 'set'; vaultMsg = null; }}
          >
            {$t('settings.vault_enable_master')}
          </button>
        {:else if !$vaultState.unlocked}
          <button
            type="button"
            class="rounded-full bg-accent px-3.5 py-1.5 text-xs font-medium text-accent-fg"
            onclick={() => { vaultAction = 'unlock'; vaultMsg = null; }}
          >
            {$t('settings.vault_unlock_now')}
          </button>
        {:else}
          <button
            type="button"
            class="rounded-full border border-strong px-3.5 py-1.5 text-xs text-fg transition hover:bg-accent hover:text-accent-fg"
            onclick={() => { vaultAction = 'set'; vaultMsg = null; }}
          >
            {$t('settings.vault_change_master')}
          </button>
          <button
            type="button"
            class="rounded-full border border-default px-3.5 py-1.5 text-xs text-muted transition hover:text-fg"
            onclick={() => { vaultAction = 'remove'; vaultMsg = null; }}
          >
            {$t('settings.vault_remove_master')}
          </button>
          <button
            type="button"
            class="rounded-full border border-default px-3.5 py-1.5 text-xs text-muted transition hover:text-fg"
            onclick={() => void lockVaultNow()}
          >
            {$t('settings.vault_lock_now')}
          </button>
        {/if}
      </div>

      {#if vaultAction !== 'none'}
        <form
          class="mt-3 space-y-2.5 rounded-xl border border-default bg-surface-inset p-3"
          onsubmit={(e) => {
            e.preventDefault();
            void submitVaultAction();
          }}
        >
          {#if $vaultState.mode === 'masterPassword'}
            <input
              bind:value={vaultCurrentPw}
              type="password"
              placeholder={$t('settings.vault_current_pw')}
              class="w-full rounded-lg border border-default bg-surface px-3 py-1.5 text-xs text-fg"
            />
          {/if}
          {#if vaultAction === 'set'}
            <input
              bind:value={vaultNewPw}
              type="password"
              placeholder={$t('settings.vault_new_pw')}
              class="w-full rounded-lg border border-default bg-surface px-3 py-1.5 text-xs text-fg"
            />
            <input
              bind:value={vaultConfirmPw}
              type="password"
              placeholder={$t('settings.vault_confirm_pw')}
              class="w-full rounded-lg border border-default bg-surface px-3 py-1.5 text-xs text-fg"
            />
          {/if}
          <div class="flex justify-end gap-2">
            <button
              type="button"
              class="rounded-full px-3 py-1 text-xs text-muted hover:text-fg"
              onclick={() => (vaultAction = 'none')}
            >
              {$t('common.cancel')}
            </button>
            <button
              type="submit"
              disabled={vaultBusy}
              class="rounded-full bg-accent px-3.5 py-1 text-xs font-medium text-accent-fg disabled:opacity-50"
            >
              {$t('common.confirm')}
            </button>
          </div>
        </form>
      {/if}
      {#if vaultMsg}
        <p class="mt-2 text-xs {vaultMsg.ok ? 'text-status-ok' : 'text-status-crit'}">{vaultMsg.text}</p>
      {/if}
    </Surface>

    <!-- Encrypted Backup & WebDAV / Git Sync -->
    <Surface class="p-5">
      <h2 class="mb-1 text-sm font-semibold">{$t('settings.sync_section')}</h2>
      <p class="mb-4 text-xs text-muted">{$t('settings.sync_desc')}</p>
      <div class="space-y-4">
        <div>
          <label class="mb-1 block text-xs text-muted" for="sync-passphrase">{$t('settings.backup_passphrase')}</label>
          <input
            id="sync-passphrase"
            bind:value={syncPassphrase}
            type="password"
            placeholder={$t('settings.backup_passphrase_placeholder')}
            class="w-full rounded-xl border border-default bg-surface-inset px-3 py-2 text-sm text-fg"
          />
          <input
            bind:this={importFileInput}
            type="file"
            accept=".korabak,.json"
            class="hidden"
            onchange={handleImportFile}
          />
          <div class="mt-2.5 flex flex-wrap gap-2">
            <button
              type="button"
              disabled={!syncPassphrase.trim() || syncBusy}
              class="rounded-full border border-strong px-3.5 py-1.5 text-xs text-fg transition hover:bg-accent hover:text-accent-fg disabled:opacity-50"
              onclick={() => void handleExportBackup()}
            >
              {$t('settings.backup_export')}
            </button>
            <button
              type="button"
              disabled={!syncPassphrase.trim() || syncBusy}
              class="rounded-full border border-default px-3.5 py-1.5 text-xs text-muted transition hover:text-fg disabled:opacity-50"
              onclick={() => triggerImport(false)}
            >
              {$t('settings.backup_import_merge')}
            </button>
            <button
              type="button"
              disabled={!syncPassphrase.trim() || syncBusy}
              class="rounded-full border border-default px-3.5 py-1.5 text-xs text-muted transition hover:text-fg disabled:opacity-50"
              onclick={() => triggerImport(true)}
            >
              {$t('settings.backup_import_replace')}
            </button>
          </div>
        </div>

        <div class="space-y-3 border-t border-default pt-4">
          <p class="text-sm">{$t('settings.sync_provider')}</p>
          <div class="flex w-full items-center gap-1 rounded-xl bg-surface-inset p-1">
            {#each (['none', 'webDav', 'git'] as SyncProviderDto[]) as p (p)}
              {@const lbl = p === 'webDav' ? 'webdav' : p}
              <button
                type="button"
                class="{segFull} {segState(syncCfg.provider === p)}"
                aria-pressed={syncCfg.provider === p}
                onclick={() => (syncCfg = { ...syncCfg, provider: p })}
              >
                {$t(`settings.sync_provider_${lbl}`)}
              </button>
            {/each}
          </div>

          {#if syncCfg.provider === 'webDav'}
            <div class="space-y-2">
              <input
                bind:value={syncCfg.webdavUrl}
                type="text"
                placeholder={$t('settings.sync_webdav_url')}
                class="w-full rounded-xl border border-default bg-surface-inset px-3 py-1.5 font-mono text-xs text-fg"
              />
              <div class="grid grid-cols-2 gap-2">
                <input
                  bind:value={syncCfg.webdavUsername}
                  type="text"
                  placeholder={$t('settings.sync_webdav_user')}
                  class="rounded-xl border border-default bg-surface-inset px-3 py-1.5 text-xs text-fg"
                />
                <input
                  bind:value={syncWebdavPassword}
                  type="password"
                  placeholder={syncCfg.hasWebdavAuth ? $t('settings.sync_webdav_pass_keep') : $t('settings.sync_webdav_pass')}
                  class="rounded-xl border border-default bg-surface-inset px-3 py-1.5 text-xs text-fg"
                />
              </div>
              <input
                bind:value={syncCfg.webdavRemotePath}
                type="text"
                placeholder={$t('settings.sync_webdav_path')}
                class="w-full rounded-xl border border-default bg-surface-inset px-3 py-1.5 font-mono text-xs text-fg"
              />
            </div>
          {:else if syncCfg.provider === 'git'}
            <div class="space-y-2">
              <input
                bind:value={syncCfg.gitRepoDir}
                type="text"
                placeholder={$t('settings.sync_git_dir')}
                class="w-full rounded-xl border border-default bg-surface-inset px-3 py-1.5 font-mono text-xs text-fg"
              />
              <div class="grid grid-cols-3 gap-2">
                <input
                  bind:value={syncCfg.gitRemote}
                  type="text"
                  placeholder={$t('settings.sync_git_remote')}
                  class="rounded-xl border border-default bg-surface-inset px-3 py-1.5 font-mono text-xs text-fg"
                />
                <input
                  bind:value={syncCfg.gitBranch}
                  type="text"
                  placeholder={$t('settings.sync_git_branch')}
                  class="rounded-xl border border-default bg-surface-inset px-3 py-1.5 font-mono text-xs text-fg"
                />
                <input
                  bind:value={syncCfg.gitFileName}
                  type="text"
                  placeholder={$t('settings.sync_git_file')}
                  class="rounded-xl border border-default bg-surface-inset px-3 py-1.5 font-mono text-xs text-fg"
                />
              </div>
            </div>
          {/if}

          <div class="flex flex-wrap gap-2">
            <button
              type="button"
              disabled={syncBusy}
              class="rounded-full border border-default px-3.5 py-1.5 text-xs text-fg transition hover:bg-surface-inset disabled:opacity-50"
              onclick={() => void handleSaveSync()}
            >
              {$t('settings.sync_save_config')}
            </button>
            {#if syncCfg.provider !== 'none'}
              <button
                type="button"
                disabled={!syncPassphrase.trim() || syncBusy}
                class="rounded-full bg-accent px-3.5 py-1.5 text-xs font-medium text-accent-fg disabled:opacity-50"
                onclick={() => void handleSyncPush()}
              >
                {$t('settings.sync_push')}
              </button>
              <button
                type="button"
                disabled={!syncPassphrase.trim() || syncBusy}
                class="rounded-full border border-strong px-3.5 py-1.5 text-xs text-fg transition hover:bg-accent hover:text-accent-fg disabled:opacity-50"
                onclick={() => void handleSyncPull()}
              >
                {$t('settings.sync_pull')}
              </button>
            {/if}
          </div>
        </div>

        {#if syncMsg}
          <p class="text-xs {syncMsg.ok ? 'text-status-ok' : 'text-status-crit'}">{syncMsg.text}</p>
        {/if}
      </div>
    </Surface>

    <!-- Updates -->
    <Surface class="p-5">
      <h2 class="mb-3 text-sm font-semibold">{$t('settings.updates')}</h2>
      <div class="space-y-4">
        <div class="flex items-center justify-between gap-4">
          <div>
            <p class="text-sm">{$t('settings.check_startup')}</p>
            <p class="text-xs text-muted">{$t('settings.check_startup_desc')}</p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={updateConfig?.checkOnStartup ?? false}
            aria-label={$t('settings.check_startup')}
            disabled={!updateConfig}
            onclick={toggleCheckOnStartup}
            class="relative h-6 w-11 shrink-0 rounded-full transition disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus {updateConfig?.checkOnStartup
              ? 'bg-accent'
              : 'bg-surface-inset'}"
          >
            <span
              class="absolute top-0.5 h-5 w-5 rounded-full bg-surface shadow-soft transition-[left] {updateConfig?.checkOnStartup
                ? 'left-[1.375rem]'
                : 'left-0.5'}"
            ></span>
          </button>
        </div>

        <div class="flex items-center justify-between gap-4 border-t border-default pt-4">
          <div class="min-w-0">
            <p class="text-sm">{$t('settings.check_now')}</p>
            <p class="text-xs text-muted">
              {#if check.kind === 'checking'}{$t('settings.checking')}
              {:else if check.kind === 'upToDate'}{$t('settings.up_to_date')}
              {:else if check.kind === 'available'}{$t('settings.update_available', { version: check.version })}
              {:else if check.kind === 'error'}{check.message}
              {:else}{$t('settings.check_startup_desc')}
              {/if}
            </p>
          </div>
          <button
            type="button"
            class="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-strong px-4 py-1.5 text-sm text-fg transition hover:bg-accent hover:text-accent-fg disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
            disabled={check.kind === 'checking'}
            onclick={checkNow}
          >
            <Icon name="refresh" size={14} />
            {$t('settings.check_now')}
          </button>
        </div>
      </div>
    </Surface>

    <!-- About: selectable, so the version can be pasted into a bug report -->
    <Surface class="p-5">
      <h2 class="mb-3 text-sm font-semibold">{$t('settings.about')}</h2>
      <div class="space-y-2">
        {#if version}
          <div class="flex items-center justify-between gap-4">
            <p class="text-sm">{$t('settings.version')}</p>
            <p class="select-text text-sm tabular-nums text-muted">{version}</p>
          </div>
        {/if}
        <p class="text-xs text-muted">{$t('settings.origin_notice')}</p>
      </div>
    </Surface>
  </div>
</section>

{#snippet traySwitch(
  label: string,
  hint: string,
  on: boolean,
  enabled: boolean,
  toggle: () => void
)}
  <div class="flex items-center justify-between gap-4">
    <div class="min-w-0">
      <p class="text-sm">{label}</p>
      <p class="text-xs text-muted">{hint}</p>
    </div>
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={!enabled}
      onclick={toggle}
      class="relative h-6 w-11 shrink-0 rounded-full transition disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus {on
        ? 'bg-accent'
        : 'bg-surface-inset'}"
    >
      <span
        class="absolute top-0.5 h-5 w-5 rounded-full bg-surface shadow-soft transition-[left] {on
          ? 'left-[1.375rem]'
          : 'left-0.5'}"
      ></span>
    </button>
  </div>
{/snippet}
