<script lang="ts">
  // One side of the dual-pane SFTP browser (tech-gui.md §3.2): an editable current-path
  // header with quick-access buttons and a parent-supplied toolbar, then the entry list.
  // Clicking a directory (or the `..` row) navigates; clicking a file previews; the
  // leading checkbox marks it for a batch transfer/delete. A press on a row may start
  // a drag (the parent owns the drag and marks the pane `dropActive` while it is the target); `data-pane` and
  // `data-dir-path` are how the parent finds the pane and folder under the pointer.
  // A listing on its way dims the current one: a remote one can wait behind a transfer.
  // Semantic tokens only — no colour literals (§5.1).
  import type { Snippet } from 'svelte';
  import { Icon, type IconName } from '$lib/theme';
  import type { FileEntryDto } from '$lib/bindings';
  import { formatBytes, formatDate, type Pane, type PaneSide } from '$lib/stores/sftp';
  import { formatPermissions, formatOctalMode } from '$lib/stores/vault';
  import { maskText } from '$lib/stores/streamer';
  import { t } from '$lib/i18n';

  export type ShortcutLocation = {
    label: string;
    path: string;
    icon?: IconName;
  };

  let {
    title,
    side,
    pane,
    shortcuts = [],
    isWindows = false,
    dropActive = false,
    dropDir,
    onNavigate,
    onNavigatePath,
    onToggleMark,
    onPreview,
    onEdit,
    onChmod,
    onDragStart,
    toolbar
  }: {
    title: string;
    side: PaneSide;
    pane: Pane;
    shortcuts?: ShortcutLocation[];
    isWindows?: boolean;
    /** The pane is the target of a drag in progress. */
    dropActive?: boolean;
    /** The folder row under the pointer while this pane is the drop target. */
    dropDir?: string;
    onNavigate: (entry: FileEntryDto) => void;
    onNavigatePath?: (path: string) => void;
    onToggleMark: (path: string) => void;
    onPreview: (entry: FileEntryDto) => void;
    onEdit?: (entry: FileEntryDto) => void;
    onChmod?: (entry: FileEntryDto) => void;
    onDragStart?: (entry: FileEntryDto, event: PointerEvent) => void;
    toolbar?: Snippet;
  } = $props();

  let isEditing = $state(false);
  let editValue = $state('');
  let pathInput = $state<HTMLInputElement>();
  let filterQuery = $state('');
  let showHidden = $state(false);

  let hideLnk = $state(
    typeof window !== 'undefined'
      ? (localStorage.getItem('korassh.sftp.hide_lnk') ?? 'true') === 'true'
      : true
  );

  function toggleHideLnk(): void {
    hideLnk = !hideLnk;
    if (typeof window !== 'undefined') {
      try {
        localStorage.setItem('korassh.sftp.hide_lnk', String(hideLnk));
      } catch {}
    }
  }

  const isWin = $derived(
    side === 'local' && (isWindows ||
      (pane.path ? pane.path.includes('\\') || /^[a-zA-Z]:/.test(pane.path) : false) ||
      (typeof navigator !== 'undefined' && /win/i.test(navigator.platform || navigator.userAgent)))
  );

  const lnkFilesCount = $derived(
    pane.entries.filter((e) => !e.isDir && e.name.toLowerCase().endsWith('.lnk')).length
  );

  const visibleEntries = $derived.by(() => {
    const q = filterQuery.trim().toLowerCase();
    return pane.entries.filter((e) => {
      if (e.name === '..') return true;
      if (!showHidden && e.name.startsWith('.')) return false;
      if (isWin && hideLnk && !e.isDir && e.name.toLowerCase().endsWith('.lnk')) return false;
      if (q && !e.name.toLowerCase().includes(q)) return false;
      return true;
    });
  });

  function startEditing(): void {
    editValue = pane.path || '';
    isEditing = true;
    requestAnimationFrame(() => {
      pathInput?.focus();
      pathInput?.select();
    });
  }

  function cancelEditing(): void {
    isEditing = false;
  }

  function submitEditing(): void {
    const trimmed = editValue.trim();
    if (trimmed && onNavigatePath) {
      onNavigatePath(trimmed);
    }
    isEditing = false;
  }

  function normalize(p: string): string {
    return p.replace(/[\\/]+$/, '').toLowerCase();
  }

  function isSameLocation(a?: string, b?: string): boolean {
    if (!a || !b) return false;
    return normalize(a) === normalize(b);
  }

  const rowBase =
    'flex w-full min-w-0 items-center gap-2 rounded px-2 py-1.5 text-left text-sm transition ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
  // The date column appears once the pane is wide enough to keep names readable. It
  // never wraps: under GDK_DPI_SCALE the rem it is sized in can shrink below its text.
  const modifiedCol = 'hidden min-w-36 shrink-0 whitespace-nowrap text-right lg:inline';
</script>

<section
  aria-label={title}
  aria-busy={pane.loading}
  data-pane={side}
  class="flex min-h-0 min-w-0 flex-1 flex-col transition
    {dropActive ? 'bg-surface-inset ring-2 ring-inset ring-accent' : ''}"
>
  <header class="shrink-0 border-b border-default px-3 py-2.5">
    <div class="flex items-center justify-between gap-2">
      <h2
        title={title}
        class="truncate text-xs font-semibold uppercase tracking-[0.14em] text-muted"
      >
        {title}
      </h2>
      <div class="flex shrink-0 items-center gap-1">
        <button
          type="button"
          class="inline-flex items-center gap-1 rounded-full border px-2 py-1 text-xs font-medium transition focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus {showHidden
            ? 'border-accent bg-surface-inset text-fg'
            : 'border-default text-muted hover:border-strong hover:bg-surface-inset hover:text-fg'}"
          title={showHidden ? $t('sftp.hide_hidden') : $t('sftp.show_hidden')}
          aria-label={showHidden ? $t('sftp.hide_hidden') : $t('sftp.show_hidden')}
          aria-pressed={showHidden}
          onclick={() => (showHidden = !showHidden)}
        >
          <Icon name={showHidden ? 'eye' : 'eyeOff'} size={13} />
          <span>.*</span>
        </button>
        {#if isWin}
          <button
            type="button"
            class="inline-flex items-center gap-1 rounded-full border px-2 py-1 text-xs font-medium transition focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus {hideLnk
              ? 'border-accent/40 bg-accent/15 text-accent font-semibold'
              : 'border-default text-muted hover:border-strong hover:bg-surface-inset hover:text-fg'}"
            title={hideLnk ? $t('sftp.show_lnk_tooltip') : $t('sftp.hide_lnk_tooltip')}
            onclick={toggleHideLnk}
          >
            <Icon name={hideLnk ? 'eyeOff' : 'eye'} size={13} />
            <span>.lnk</span>
            {#if lnkFilesCount > 0}
              <span class="ml-0.5 rounded px-1 text-[10px] opacity-75 {hideLnk ? 'bg-accent/20' : 'bg-surface-inset'}">{lnkFilesCount}</span>
            {/if}
          </button>
        {/if}
        {@render toolbar?.()}
      </div>
    </div>

    <!-- Path Bar (editable address bar) -->
    {#if isEditing}
      <form
        class="mt-2 flex items-center gap-1.5"
        onsubmit={(e) => {
          e.preventDefault();
          submitEditing();
        }}
      >
        <div class="relative flex min-w-0 flex-1 items-center">
          <input
            bind:this={pathInput}
            bind:value={editValue}
            type="text"
            class="w-full rounded-md border border-accent bg-surface-inset px-2.5 py-1 font-mono text-xs text-fg outline-none focus:ring-1 focus:ring-accent"
            placeholder={$t('sftp.path_placeholder')}
            onkeydown={(e) => {
              if (e.key === 'Escape') cancelEditing();
            }}
          />
        </div>
        <button
          type="submit"
          class="grid h-7 w-7 shrink-0 place-items-center rounded-md border border-default text-muted transition hover:border-strong hover:bg-surface-inset hover:text-fg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus"
          title={$t('sftp.go_to_path')}
          aria-label={$t('sftp.go_to_path')}
        >
          <Icon name="check" size={13} />
        </button>
        <button
          type="button"
          class="grid h-7 w-7 shrink-0 place-items-center rounded-md border border-default text-muted transition hover:border-strong hover:bg-surface-inset hover:text-fg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus"
          title={$t('sftp.cancel')}
          aria-label={$t('sftp.cancel')}
          onclick={cancelEditing}
        >
          <Icon name="close" size={13} />
        </button>
      </form>
    {:else}
      <div
        role="button"
        tabindex="0"
        class="group mt-2 flex items-center justify-between gap-2 rounded-md border border-default/40 bg-surface-inset/50 px-2.5 py-1 font-mono text-xs text-muted transition hover:border-default hover:bg-surface-inset cursor-pointer focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus"
        title={$t('sftp.click_to_edit_path')}
        onclick={startEditing}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            startEditing();
          }
        }}
      >
        <div class="flex min-w-0 items-center gap-1.5 flex-1">
          <span class="text-faint transition group-hover:text-muted shrink-0">
            <Icon name="folder" size={13} />
          </span>
          <span class="truncate text-fg" title={pane.path}>
            {pane.path || '—'}
          </span>
        </div>
        <span
          class="shrink-0 text-faint opacity-50 transition group-hover:opacity-100 group-hover:text-muted"
          title={$t('sftp.click_to_edit_path')}
        >
          <Icon name="edit" size={12} />
        </span>
      </div>
    {/if}

    <!-- Filter box -->
    <div class="mt-2 flex items-center gap-1.5">
      <div class="relative flex min-w-0 flex-1 items-center">
        <span class="pointer-events-none absolute left-2.5 text-faint">
          <Icon name="search" size={12} />
        </span>
        <input
          bind:value={filterQuery}
          type="text"
          placeholder={$t('sftp.filter_placeholder')}
          aria-label={$t('sftp.filter_placeholder')}
          class="w-full rounded-md border border-default bg-surface-inset py-1 pl-7 pr-6 text-xs text-fg placeholder:text-faint focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus"
        />
        {#if filterQuery}
          <button
            type="button"
            class="absolute right-1.5 text-faint hover:text-fg"
            aria-label={$t('common.close')}
            onclick={() => (filterQuery = '')}
          >
            <Icon name="close" size={11} />
          </button>
        {/if}
      </div>
    </div>

    <!-- Quick Access Location Buttons -->
    {#if shortcuts.length > 0}
      <div class="mt-2 flex flex-wrap items-center gap-1.5">
        <!-- XDG locations can share a path (including Home and Root at /). -->
        {#each shortcuts as sc (`${sc.label}\u0000${sc.path}`)}
          {@const active = isSameLocation(pane.path, sc.path)}
          <button
            type="button"
            class="inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-[11px] font-medium transition focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus
              {active
                ? 'border border-accent/40 bg-accent/15 text-accent font-semibold shadow-sm'
                : 'border border-default/70 text-muted hover:border-strong hover:bg-surface-inset hover:text-fg'}"
            title={sc.path}
            onclick={() => onNavigatePath?.(sc.path)}
          >
            {#if sc.icon}<Icon name={sc.icon} size={11} />{/if}
            <span>{sc.label}</span>
          </button>
        {/each}
      </div>
    {/if}
    {#if pane.loading && pane.entries.length > 0}
      <p class="mt-1 text-xs text-faint">{$t('sftp.loading')}</p>
    {/if}
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto px-1.5 py-1.5">
    {#if pane.error}
      <p class="px-2 py-6 text-center text-sm text-status-crit">{$maskText(pane.error)}</p>
    {:else if pane.loading && pane.entries.length === 0}
      <p class="px-2 py-6 text-center text-sm text-faint">{$t('sftp.loading')}</p>
    {:else if visibleEntries.length === 0}
      <p class="px-2 py-6 text-center text-sm text-faint">{$t('sftp.empty_directory')}</p>
    {:else}
      <div
        class="flex items-center gap-2 px-2 pb-1 pl-[2.125rem] text-[0.65rem] font-semibold uppercase
          tracking-[0.12em] text-faint"
        aria-hidden="true"
      >
        <span class="min-w-0 flex-1 pl-[1.4rem]">{$t('sftp.column_name')}</span>
        <span class="hidden w-20 shrink-0 text-right font-mono xl:inline">{$t('sftp.column_perms')}</span>
        <span class="shrink-0">{$t('sftp.column_size')}</span>
        <span class={modifiedCol}>{$t('sftp.column_modified')}</span>
      </div>
      <ul class="space-y-0.5 transition-opacity {pane.loading ? 'opacity-50' : ''}">
        {#each visibleEntries as entry, i (i)}
          {@const isParent = entry.name === '..'}
          {@const marked = pane.marked.has(entry.path)}
          {@const dropHere = dropActive && entry.isDir && dropDir === entry.path}
          <li
            class="group/row flex items-center gap-1.5 rounded {dropHere ? 'ring-2 ring-accent' : ''}"
            data-dir-path={entry.isDir ? entry.path : undefined}
          >
            {#if isParent}
              <span class="h-4 w-4 shrink-0"></span>
            {:else}
              <button
                type="button"
                role="checkbox"
                aria-checked={marked}
                aria-label={$t('sftp.mark_entry', { name: entry.name })}
                class="grid h-4 w-4 shrink-0 place-items-center rounded border transition
                  focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus
                  {marked ? 'border-accent bg-accent text-accent-fg' : 'border-strong text-transparent'}"
                onclick={() => onToggleMark(entry.path)}
              >
                {#if marked}<Icon name="check" size={11} />{/if}
              </button>
            {/if}
            <button
              type="button"
              class="{rowBase} text-muted hover:bg-surface-inset hover:text-fg"
              title={entry.name}
              onclick={() => (entry.isDir ? onNavigate(entry) : onPreview(entry))}
              onpointerdown={(e) => onDragStart?.(entry, e)}
            >
              <Icon name={entry.isDir ? 'folder' : 'file'} size={15} />
              <span class="min-w-0 flex-1 truncate {entry.isDir ? 'font-medium text-fg' : ''}">
                {entry.name}
              </span>
              {#if !isParent && entry.permissions != null}
                <span
                  class="hidden w-20 shrink-0 text-right font-mono text-[11px] text-faint xl:inline"
                  title={formatOctalMode(entry.permissions)}
                >
                  {formatPermissions(entry.permissions)}
                </span>
              {/if}
              {#if !entry.isDir}
                <span class="shrink-0 tabular-nums text-xs text-faint">{formatBytes(entry.size)}</span>
              {/if}
              {#if !isParent}
                <span
                  class="{modifiedCol} tabular-nums text-xs text-faint"
                  title={entry.modified != null ? $t('sftp.modified_at', { date: formatDate(entry.modified, true) }) : undefined}
                >
                  {formatDate(entry.modified)}
                </span>
              {/if}
            </button>
            {#if !isParent && side === 'remote'}
              <div class="flex shrink-0 items-center gap-0.5 opacity-0 transition group-hover/row:opacity-100 focus-within:opacity-100">
                {#if !entry.isDir && onEdit}
                  <button
                    type="button"
                    class="grid h-6 w-6 place-items-center rounded text-muted transition hover:bg-surface-inset hover:text-fg"
                    title={$t('sftp.edit_file')}
                    aria-label={$t('sftp.edit_file')}
                    onclick={() => onEdit(entry)}
                  >
                    <Icon name="edit" size={12} />
                  </button>
                {/if}
                {#if onChmod}
                  <button
                    type="button"
                    class="grid h-6 w-6 place-items-center rounded text-muted transition hover:bg-surface-inset hover:text-fg"
                    title={$t('sftp.chmod')}
                    aria-label={$t('sftp.chmod')}
                    onclick={() => onChmod(entry)}
                  >
                    <Icon name="shield" size={12} />
                  </button>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</section>
