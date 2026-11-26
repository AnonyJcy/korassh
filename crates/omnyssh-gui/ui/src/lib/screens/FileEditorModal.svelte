<script lang="ts">
  import Modal from '$lib/components/Modal.svelte';
  import { Button, Icon } from '$lib/theme';
  import { t } from '$lib/i18n';

  let {
    path,
    initialContent,
    loading = false,
    error,
    saving = false,
    onSave,
    onClose
  }: {
    path: string;
    initialContent: string;
    loading?: boolean;
    error?: string;
    saving?: boolean;
    onSave: (content: string) => void;
    onClose: () => void;
  } = $props();

  let draft = $state('');
  let loadedPath = $state('');
  let loadedSource = $state('');

  $effect(() => {
    if (!loading && (path !== loadedPath || initialContent !== loadedSource)) {
      draft = initialContent;
      loadedPath = path;
      loadedSource = initialContent;
    }
  });

  const dirty = $derived(!loading && !error && draft !== loadedSource);
  const lineCount = $derived(draft ? draft.split('\n').length : 1);

  function handleKeydown(e: KeyboardEvent): void {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
      e.preventDefault();
      if (!loading && !error && !saving) onSave(draft);
    }
  }
</script>

<Modal onClose={onClose} label={$t('sftp.editor_title')} wide>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="flex flex-col gap-3 p-5" onkeydown={handleKeydown}>
    <header class="flex items-center justify-between gap-3">
      <div class="min-w-0">
        <div class="flex items-center gap-2">
          <h2 class="text-sm font-semibold">{$t('sftp.editor_title')}</h2>
          {#if dirty}
            <span class="rounded-full bg-surface-inset px-2 py-0.5 text-[11px] text-status-warn">
              {$t('sftp.editor_unsaved')}
            </span>
          {/if}
        </div>
        <p class="mt-0.5 truncate font-mono text-xs text-muted" title={path}>{path}</p>
      </div>
      <div class="flex shrink-0 items-center gap-2">
        <Button
          variant="primary"
          disabled={loading || !!error || saving}
          onclick={() => onSave(draft)}
        >
          <Icon name="save" size={14} />
          {saving ? $t('sftp.editor_saving') : $t('sftp.editor_save')}
        </Button>
        <Button variant="ghost" onclick={onClose}>{$t('common.close')}</Button>
      </div>
    </header>

    {#if loading}
      <div class="flex h-80 items-center justify-center rounded-xl border border-default bg-surface-inset text-sm text-muted">
        {$t('sftp.editor_loading')}
      </div>
    {:else if error}
      <div class="rounded-xl border border-default bg-surface-inset p-4 text-sm text-status-crit">
        {error}
      </div>
    {:else}
      <textarea
        bind:value={draft}
        spellcheck="false"
        class="h-96 w-full resize-y rounded-xl border border-default bg-surface-inset p-3 font-mono text-xs leading-relaxed text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
      ></textarea>
      <div class="flex items-center justify-between text-[11px] text-faint">
        <span>UTF-8 · {lineCount} lines</span>
        <span>Ctrl+S / ⌘S</span>
      </div>
    {/if}
  </div>
</Modal>
