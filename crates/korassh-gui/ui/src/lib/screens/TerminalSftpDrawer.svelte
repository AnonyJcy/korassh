<script lang="ts">
  // Collapsible remote SFTP drawer docked inside a terminal session.
  // Opens its own SFTP session lazily when expanded and cleanly closes it on unmount.
  import { onDestroy, onMount } from 'svelte';
  import type { FileEntryDto } from '$lib/bindings';
  import { Icon } from '$lib/theme';
  import { sftp, formatBytes } from '$lib/stores/sftp';
  import { formatOctalMode, formatPermissions } from '$lib/stores/vault';
  import {
    sftpOpen,
    sftpList,
    sftpClose,
    sftpReadFile,
    sftpWriteFile,
    sftpChmod,
    sftpMkdir,
    sftpDelete
  } from '$lib/ipc/commands';
  import { displayHostTitle, maskText, streamerMode } from '$lib/stores/streamer';
  import { lastError } from '$lib/stores/notifications';
  import Modal from '$lib/components/Modal.svelte';
  import FileEditorModal from './FileEditorModal.svelte';
  import { t } from '$lib/i18n';

  let { hostName, onClose }: { hostName: string; onClose: () => void } = $props();

  let backendId = $state<number | undefined>();
  let destroyed = false;
  let openError = $state<string | undefined>();
  let filterQuery = $state('');
  let showHidden = $state(false);
  let pathInput = $state('/');
  let editingPath = $state(false);
  let savingEditor = $state(false);
  let chmodTarget = $state<{ entry: FileEntryDto; value: string; error?: string } | null>(null);
  let mkdirOpen = $state(false);
  let mkdirName = $state('');

  const view = $derived(backendId == null ? undefined : $sftp.get(backendId));

  const visibleEntries = $derived.by(() => {
    const entries = view?.remote.entries ?? [];
    const q = filterQuery.trim().toLowerCase();
    return entries.filter((e) => {
      if (e.name === '..') return true;
      if (!showHidden && e.name.startsWith('.')) return false;
      if (q && !e.name.toLowerCase().includes(q)) return false;
      return true;
    });
  });

  function joinRemote(dir: string, name: string): string {
    return dir.endsWith('/') ? `${dir}${name}` : `${dir}/${name}`;
  }

  function listPath(path: string): void {
    if (backendId == null) return;
    sftp.beginLoading(backendId, 'remote');
    void sftpList(backendId, path).catch((err) => {
      if (backendId != null) sftp.paneError(backendId, 'remote', err instanceof Error ? err.message : String(err));
    });
  }

  $effect(() => {
    if (view?.remote.path && !editingPath) {
      pathInput = view.remote.path;
    }
  });

  $effect(() => {
    if (backendId == null || !view || view.pending.length > 0 || !view.refresh) return;
    sftp.clearRefresh(backendId);
    savingEditor = false;
    listPath(view.remote.path || '/');
  });

  onMount(() => {
    void (async () => {
      if (typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window)) return;
      try {
        const id = await sftpOpen(hostName);
        if (destroyed) {
          void sftpClose(id).catch(() => {});
          return;
        }
        backendId = id;
        sftp.open(id, hostName);
        listPath('/');
      } catch (err) {
        openError = err instanceof Error ? err.message : String(err);
      }
    })();
  });

  onDestroy(() => {
    destroyed = true;
    if (backendId != null) {
      void sftpClose(backendId).catch(() => {});
      sftp.remove(backendId);
    }
  });

  function openEdit(entry: FileEntryDto): void {
    if (backendId == null || entry.isDir) return;
    sftp.beginEdit(backendId, entry.path);
    void sftpReadFile(backendId, entry.path).catch((err) => {
      if (backendId != null) {
        sftp.setEditorContent(backendId, entry.path, null, err instanceof Error ? err.message : String(err));
      }
    });
  }

  function saveEdit(content: string): void {
    if (backendId == null || !view?.editor) return;
    savingEditor = true;
    sftp.pushOp(backendId, { kind: 'write', refresh: 'remote' });
    void sftpWriteFile(backendId, view.editor.path, content)
      .then(() => {
        if (backendId != null) sftp.clearEditor(backendId);
      })
      .catch((err) => {
        savingEditor = false;
        lastError.set(err instanceof Error ? err.message : String(err));
      });
  }

  function applyChmod(): void {
    if (backendId == null || !chmodTarget) return;
    const raw = chmodTarget.value.trim();
    if (!/^[0-7]{3,4}$/.test(raw)) {
      chmodTarget = { ...chmodTarget, error: $t('sftp.chmod_invalid') };
      return;
    }
    const mode = parseInt(raw, 8);
    const targetPath = chmodTarget.entry.path;
    chmodTarget = null;
    sftp.pushOp(backendId, { kind: 'chmod', refresh: 'remote' });
    void sftpChmod(backendId, targetPath, mode).catch((err) =>
      lastError.set(err instanceof Error ? err.message : String(err))
    );
  }

  function createDir(): void {
    if (backendId == null || !view) return;
    const name = mkdirName.trim();
    if (!name) return;
    mkdirName = '';
    mkdirOpen = false;
    sftp.pushOp(backendId, { kind: 'mkdir', refresh: 'remote' });
    void sftpMkdir(backendId, joinRemote(view.remote.path || '/', name)).catch((err) =>
      lastError.set(err instanceof Error ? err.message : String(err))
    );
  }

  function deleteEntry(entry: FileEntryDto): void {
    if (backendId == null || entry.name === '..') return;
    sftp.pushOp(backendId, { kind: 'delete', refresh: 'remote' });
    void sftpDelete(backendId, entry.path).catch((err) =>
      lastError.set(err instanceof Error ? err.message : String(err))
    );
  }
</script>

<aside
  aria-label={$t('sftp.sidebar_title', { host: displayHostTitle(hostName, $streamerMode) })}
  class="flex h-full w-80 shrink-0 flex-col border-l border-default bg-surface"
>
  <header class="shrink-0 space-y-2 border-b border-default px-3 py-2">
    <div class="flex items-center justify-between gap-2">
      <div class="flex min-w-0 items-center gap-1.5 text-xs font-semibold text-fg">
        <Icon name="sftp" size={14} />
        <span class="truncate">{$t('sftp.sidebar_title', { host: displayHostTitle(hostName, $streamerMode) })}</span>
      </div>
      <div class="flex shrink-0 items-center gap-1">
        <button
          type="button"
          class="grid h-6 w-6 place-items-center rounded text-muted transition hover:bg-surface-inset hover:text-fg"
          title={showHidden ? $t('sftp.hide_hidden') : $t('sftp.show_hidden')}
          onclick={() => (showHidden = !showHidden)}
        >
          <Icon name={showHidden ? 'eye' : 'eyeOff'} size={13} />
        </button>
        <button
          type="button"
          class="grid h-6 w-6 place-items-center rounded text-muted transition hover:bg-surface-inset hover:text-fg"
          title={$t('sftp.new_folder')}
          onclick={() => (mkdirOpen = !mkdirOpen)}
        >
          <Icon name="plus" size={13} />
        </button>
        <button
          type="button"
          class="grid h-6 w-6 place-items-center rounded text-muted transition hover:bg-surface-inset hover:text-fg"
          title={$t('sftp.refresh')}
          onclick={() => listPath(view?.remote.path || '/')}
        >
          <Icon name="refresh" size={13} />
        </button>
        <button
          type="button"
          class="grid h-6 w-6 place-items-center rounded text-muted transition hover:bg-surface-inset hover:text-fg"
          title={$t('common.close')}
          onclick={onClose}
        >
          <Icon name="close" size={13} />
        </button>
      </div>
    </div>

    <form
      class="flex items-center gap-1"
      onsubmit={(e) => {
        e.preventDefault();
        editingPath = false;
        listPath(pathInput.trim() || '/');
      }}
    >
      <input
        bind:value={pathInput}
        onfocus={() => (editingPath = true)}
        onblur={() => (editingPath = false)}
        type="text"
        class="w-full rounded-md border border-default bg-surface-inset px-2 py-1 font-mono text-[11px] text-fg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus"
      />
    </form>

    {#if mkdirOpen}
      <form
        class="flex items-center gap-1"
        onsubmit={(e) => {
          e.preventDefault();
          createDir();
        }}
      >
        <input
          bind:value={mkdirName}
          type="text"
          placeholder={$t('sftp.new_folder')}
          class="w-full rounded-md border border-default bg-surface-inset px-2 py-1 text-xs text-fg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus"
        />
        <button
          type="submit"
          class="rounded-md bg-accent px-2 py-1 text-xs text-accent-fg"
        >
          {$t('common.confirm')}
        </button>
      </form>
    {/if}

    <input
      bind:value={filterQuery}
      type="text"
      placeholder={$t('sftp.filter_placeholder')}
      class="w-full rounded-md border border-default bg-surface-inset px-2 py-1 text-xs text-fg placeholder:text-faint focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus"
    />
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto px-1.5 py-1.5">
    {#if openError}
      <p class="p-3 text-xs text-status-crit">{$maskText(openError)}</p>
    {:else if !view || (view.remote.loading && view.remote.entries.length === 0)}
      <p class="p-3 text-center text-xs text-faint">{$t('sftp.loading')}</p>
    {:else if view.remote.error}
      <p class="p-3 text-xs text-status-crit">{$maskText(view.remote.error)}</p>
    {:else if visibleEntries.length === 0}
      <p class="p-3 text-center text-xs text-faint">{$t('sftp.empty_directory')}</p>
    {:else}
      <ul class="space-y-0.5">
        {#each visibleEntries as entry, i (i)}
          {@const isParent = entry.name === '..'}
          <li class="group flex items-center gap-1 rounded px-1.5 py-1 text-xs hover:bg-surface-inset">
            <button
              type="button"
              class="flex min-w-0 flex-1 items-center gap-1.5 text-left"
              title={entry.path}
              onclick={() => (entry.isDir ? listPath(entry.path) : openEdit(entry))}
            >
              <Icon name={entry.isDir ? 'folder' : 'file'} size={13} />
              <span class="min-w-0 flex-1 truncate {entry.isDir ? 'font-medium text-fg' : 'text-muted'}">
                {entry.name}
              </span>
              {#if !entry.isDir}
                <span class="shrink-0 tabular-nums text-[10px] text-faint">{formatBytes(entry.size)}</span>
              {/if}
            </button>
            {#if !isParent}
              <div class="flex shrink-0 items-center gap-0.5 opacity-0 transition group-hover:opacity-100">
                {#if !entry.isDir}
                  <button
                    type="button"
                    class="grid h-5 w-5 place-items-center rounded text-muted hover:text-fg"
                    title={$t('sftp.edit_file')}
                    onclick={() => openEdit(entry)}
                  >
                    <Icon name="edit" size={11} />
                  </button>
                {/if}
                <button
                  type="button"
                  class="rounded px-1 font-mono text-[10px] text-faint hover:text-fg"
                  title={$t('sftp.chmod')}
                  onclick={() => (chmodTarget = { entry, value: formatOctalMode(entry.permissions) })}
                >
                  {entry.permissions != null ? formatPermissions(entry.permissions) : 'chmod'}
                </button>
                <button
                  type="button"
                  class="grid h-5 w-5 place-items-center rounded text-muted hover:text-status-crit"
                  title={$t('sftp.delete')}
                  onclick={() => deleteEntry(entry)}
                >
                  <Icon name="trash" size={11} />
                </button>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</aside>

{#if view?.editor}
  <FileEditorModal
    path={view.editor.path}
    initialContent={view.editor.content}
    loading={view.editor.loading}
    error={view.editor.error}
    saving={savingEditor}
    onSave={saveEdit}
    onClose={() => backendId != null && sftp.clearEditor(backendId)}
  />
{/if}

{#if chmodTarget}
  <Modal onClose={() => (chmodTarget = null)} label={$t('sftp.chmod_title')}>
    <form
      class="space-y-4 p-5"
      onsubmit={(e) => {
        e.preventDefault();
        applyChmod();
      }}
    >
      <div>
        <h2 class="text-sm font-semibold">{$t('sftp.chmod_title')}</h2>
        <p class="mt-0.5 truncate font-mono text-xs text-muted">{chmodTarget.entry.path}</p>
      </div>
      <div>
        <label class="mb-1 block text-xs text-muted" for="drawer-chmod">{$t('sftp.chmod_octal')}</label>
        <input
          id="drawer-chmod"
          bind:value={chmodTarget.value}
          type="text"
          class="w-full rounded-xl border border-default bg-surface-inset px-3 py-2 font-mono text-sm text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
        />
        {#if chmodTarget.error}
          <p class="mt-1 text-xs text-status-crit">{chmodTarget.error}</p>
        {/if}
      </div>
      <div class="flex justify-end gap-2">
        <button
          type="button"
          class="rounded-full border border-default px-4 py-1.5 text-sm text-muted hover:text-fg"
          onclick={() => (chmodTarget = null)}
        >
          {$t('common.cancel')}
        </button>
        <button
          type="submit"
          class="rounded-full bg-accent px-4 py-1.5 text-sm font-medium text-accent-fg"
        >
          {$t('sftp.chmod_apply')}
        </button>
      </div>
    </form>
  </Modal>
{/if}
