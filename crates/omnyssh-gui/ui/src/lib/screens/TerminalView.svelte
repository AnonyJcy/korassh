<script lang="ts">
  // A live terminal tab (tech-gui.md §3.1). One instance per terminal session, kept
  // mounted for the session's whole life — hidden, not destroyed, when another entity
  // is active — so scrollback and the byte stream survive tab switches. Raw output
  // arrives on a per-session channel (§3.3/§3.6); keystrokes/resizes go back over the
  // terminal commands. Subscribes to the theme store and re-themes live (§5.1).
  import '@xterm/xterm/css/xterm.css';
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import type { Terminal } from '@xterm/xterm';
  import type { FitAddon } from '@xterm/addon-fit';
  import type { SearchAddon } from '@xterm/addon-search';
  import { Channel } from '@tauri-apps/api/core';
  import { theme } from '$lib/stores/theme';
  import { fontPrefs, resolveTerminalFontStack } from '$lib/stores/font';
  import { terminalPrefs } from '$lib/stores/terminalPrefs';
  import { xtermTheme } from '$lib/theme/terminalTheme';
  import { sessions, type Session } from '$lib/stores/sessions';
  import { snippets } from '$lib/stores/snippets';
  import { splitLayout } from '$lib/stores/layout';
  import { closeSession, duplicateSession, splitWithDuplicate } from '$lib/stores/navigation';
  import { terminalDidExit } from '$lib/ipc/router';
  import { lastError } from '$lib/stores/notifications';
  import { dialogs } from '$lib/stores/dialogs';
  import { palette } from '$lib/stores/palette';
  import { isStreamerChord } from '$lib/stores/ui';
  import { activeEntity } from '$lib/stores/activeEntity';
  import { streamerMode, displayHostTitle } from '$lib/stores/streamer';
  import { openExternal } from '$lib/ipc/openExternal';
  import { Icon } from '$lib/theme';
  import { t } from '$lib/i18n';
  import {
    terminalOpen,
    terminalOpenLocal,
    terminalWrite,
    terminalResize,
    terminalClose,
    terminalPaste,
    clipboardReadText,
    clipboardWriteText,
    listSnippets,
    saveTextFile
  } from '$lib/ipc/commands';
  import { shouldFadeTop } from './terminalFade';
  import {
    applyKeywordHighlights,
    compileHighlightRules,
    stripAnsi,
    type CompiledHighlightRule
  } from './terminalHighlight';
  import {
    bufferText,
    chunkBytes,
    closesEndedTab,
    isCopyAllShortcut,
    isCopyInsert,
    isCopyShortcut,
    isFindShortcut,
    isMultiLine,
    isPasteShortcut,
    layoutFallback,
    lineCount,
    middleClickPastes,
    rightClickAction,
    trimCopied
  } from './terminalInput';
  import { HIGHLIGHT_LIMIT, seedQuery } from './terminalFind';
  import TerminalFind from './TerminalFind.svelte';
  import TerminalSftpDrawer from './TerminalSftpDrawer.svelte';
  import { clipboardPrefs } from '$lib/stores/clipboard';
  import Modal from '$lib/components/Modal.svelte';
  import { isMac, isWindows } from '$lib/platform';
  import type { SnippetDto, TerminalBytes } from '$lib/bindings';

  let {
    session,
    active,
    paneClass = 'inset-0',
    focused = true,
    isSplit = false,
    onPaneFocus
  }: {
    session: Session;
    active: boolean;
    paneClass?: string;
    focused?: boolean;
    isSplit?: boolean;
    onPaneFocus?: () => void;
  } = $props();

  // The Nerd Font families come after the generic `monospace`, not merely after the
  // named system ones: the named list is macOS/Windows-only, so on a Linux desktop a
  // patched font ahead of the generic would become the terminal's Latin face and size
  // its cell from itself. Per-character fallback continues past a generic family, so the
  // Private Use Area glyphs (starship, powerlevel10k, eza --icons) still reach the tail.
  // Within the tail, the single-width variants come first — Nerd Fonts v3 ships icons at
  // double width in the bare family and one cell wide in its `Mono` twin.
  const MONO =
    'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace, ' +
    '"Symbols Nerd Font Mono", "Symbols Nerd Font", "MesloLGS NF", ' +
    '"JetBrainsMono Nerd Font Mono", "JetBrainsMono Nerd Font", ' +
    '"Hack Nerd Font Mono", "Hack Nerd Font", ' +
    '"FiraCode Nerd Font Mono", "FiraCode Nerd Font"';
  // One encoder for the keystroke hot path instead of one per input event.
  const ENCODER = new TextEncoder();
  const DECODER = new TextDecoder('utf-8', { fatal: false });

  // A large paste arrives as one onData; sending it as a single number[] would freeze
  // the UI thread (§9). Split into bounded chunks and await each so paint yields between
  // them; a serialization chain keeps all input strictly in order across events.
  let writeChain: Promise<void> = Promise.resolve();
  function sendInput(bytes: Uint8Array): void {
    if (termId == null || bytes.length === 0 || ended()) return;
    writeChain = writeChain.then(async () => {
      for (const chunk of chunkBytes(bytes)) {
        if (destroyed || termId == null) return;
        try {
          await terminalWrite(termId, Array.from(chunk));
        } catch {
          // Stop this input on a write failure rather than sending a gapped stream.
          return;
        }
      }
    });
  }

  /** No dialog or palette holds the keyboard: keystrokes meant for a key passphrase
   *  must never reach the shell. */
  function keyboardFree(): boolean {
    return get(dialogs).length === 0 && !get(palette).open;
  }

  /** The remote side ended the session: the tab stays for its last output, and the
   *  backend has nothing left to write or resize. Read from the store, which a late
   *  channel message can follow the exit into. */
  function ended(): boolean {
    return get(sessions).some((s) => s.id === session.id && s.status === 'closed');
  }

  let container: HTMLDivElement;
  let term: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let searchAddon = $state<SearchAddon | undefined>();
  let findOpen = $state(false);
  let findSeed = $state('');
  let findBar: TerminalFind | undefined = $state();
  let termId: number | undefined;
  let destroyed = false;
  let connected = false;
  let ready = $state(false);
  let themeUnsub: (() => void) | undefined;
  let fontUnsub: (() => void) | undefined;
  let prefsUnsub: (() => void) | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let fitScheduled = false;
  let sftpDrawerOpen = $state(false);
  let quickBarOpen = $state(false);
  let quickFilter = $state('');
  let recording = $state(false);
  let recordedChunks: string[] = [];
  let reconnecting = $state(false);
  let compiledHighlights: CompiledHighlightRule[] = [];
  let highlightActive = $state(terminalPrefs.get().highlightEnabled);
  let paramPrompt = $state<{ snippet: SnippetDto; values: Record<string, string> } | null>(null);

  const sessionEnded = $derived(session.status === 'closed' || session.status === 'failed');

  const quickSnippets = $derived.by(() => {
    const q = quickFilter.trim().toLowerCase();
    return $snippets.filter((s) => {
      if (s.scope === 'host' && s.host && s.host !== session.hostName) return false;
      if (q && !`${s.name} ${s.command}`.toLowerCase().includes(q)) return false;
      return true;
    });
  });

  function onWheel(e: WheelEvent): void {
    if (!(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    terminalPrefs.zoom(e.deltaY < 0 ? 1 : -1);
  }

  function sendQuickSnippet(snippet: SnippetDto): void {
    const params = snippet.params?.filter(Boolean) ?? [];
    if (params.length > 0) {
      const values: Record<string, string> = {};
      for (const p of params) values[p] = '';
      paramPrompt = { snippet, values };
      return;
    }
    sendInput(ENCODER.encode(`${snippet.command}\r`));
    flash($t('terminal.quick_commands_sent', { name: snippet.name }));
    term?.focus();
  }

  function submitParamSnippet(): void {
    if (!paramPrompt) return;
    let cmd = paramPrompt.snippet.command;
    for (const [k, v] of Object.entries(paramPrompt.values)) {
      cmd = cmd.replaceAll(`{{${k}}}`, v);
    }
    const name = paramPrompt.snippet.name;
    paramPrompt = null;
    sendInput(ENCODER.encode(`${cmd}\r`));
    flash($t('terminal.quick_commands_sent', { name }));
    term?.focus();
  }

  async function toggleRecording(): Promise<void> {
    if (!recording) {
      recordedChunks = [];
      recording = true;
      return;
    }
    recording = false;
    const text = recordedChunks.join('');
    recordedChunks = [];
    const stamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
    const safeHost = session.hostName.replace(/[^a-zA-Z0-9._-]/g, '_');
    const fileName = `${safeHost}-${stamp}.log`;
    if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
      try {
        const savedPath = await saveTextFile(fileName, text);
        flash($t('terminal.log_saved', { path: savedPath }));
      } catch (err) {
        lastError.set(err instanceof Error ? err.message : String(err));
      }
    } else {
      const blob = new Blob([text], { type: 'text/plain;charset=utf-8' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = fileName;
      a.click();
      URL.revokeObjectURL(url);
      flash($t('terminal.log_downloaded'));
    }
  }

  async function openBackendPty(): Promise<void> {
    if (!term || destroyed) return;
    if (typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window)) {
      ready = true;
      return;
    }
    connected = false;
    sessions.setStatus(session.id, 'connecting');
    const channel = new Channel<TerminalBytes>();
    channel.onmessage = (msg) => {
      if (!term) return;
      if (!connected) {
        connected = true;
        if (!ended()) sessions.setStatus(session.id, 'connected');
      }
      const bytes = new Uint8Array(msg as unknown as ArrayBuffer);
      const shouldHighlight =
        highlightActive &&
        compiledHighlights.length > 0 &&
        term.buffer.active.type === 'normal';
      if (recording || shouldHighlight) {
        const decoded = DECODER.decode(bytes, { stream: true });
        if (recording) recordedChunks.push(stripAnsi(decoded));
        if (shouldHighlight) {
          term.write(applyKeywordHighlights(decoded, compiledHighlights), syncScrolled);
          return;
        }
      }
      term.write(bytes, syncScrolled);
    };

    const id = session.isLocal
      ? await terminalOpenLocal(term.cols || 80, term.rows || 24, channel)
      : await terminalOpen(session.hostName, term.cols || 80, term.rows || 24, channel);
    if (destroyed) {
      void terminalClose(id).catch(() => {});
      return;
    }
    termId = id;
    sessions.setTermId(session.id, id);
    const exited = terminalDidExit(id);
    if (exited === false) {
      closeSession(session.id);
      return;
    }
    if (exited) sessions.setStatus(session.id, 'closed');
  }

  async function reconnect(): Promise<void> {
    if (reconnecting || !term) return;
    reconnecting = true;
    try {
      if (termId != null) {
        void terminalClose(termId).catch(() => {});
        termId = undefined;
      }
      term.write('\r\n\x1b[2m--- Reconnecting... ---\x1b[0m\r\n');
      await openBackendPty();
      safeFit();
      term.focus();
    } catch (err) {
      lastError.set(err instanceof Error ? err.message : String(err));
      sessions.setStatus(session.id, 'failed');
    } finally {
      reconnecting = false;
    }
  }
  // The top-edge fade dissolves scrolled output into the top edge, but never the live
  // prompt: after `clear`/Ctrl+L the cursor homes to the top, so the fade must lift
  // there (see terminalFade). Recomputed after every write too, since those resets
  // move the viewport without firing onScroll.
  let scrolled = $state(false);
  function syncScrolled(): void {
    const buf = term?.buffer.active;
    scrolled = !!buf && shouldFadeTop(buf.viewportY, buf.baseY, buf.cursorY);
  }

  // ── Copy & paste (PuTTY-style; see stores/clipboard) ──────────────────────────────
  // Both directions go through the native clipboard (commands/clipboard.rs): copy on
  // select happens outside any key gesture, which `navigator.clipboard` refuses there.
  const errText = (err: unknown): string => (err instanceof Error ? err.message : String(err));

  /** Copy the current selection, if any. Returns whether there was one. */
  function copySelection(): boolean {
    if (!term?.hasSelection()) return false;
    let text = term.getSelection();
    if (clipboardPrefs.get().trimTrailingWhitespace) text = trimCopied(text);
    if (!text) return false;
    clipboardWriteText(text).catch((err) => lastError.set(`Copy failed: ${errText(err)}`));
    return true;
  }

  /** A short note over the terminal ("Copied 312 lines"), so a copy with nothing to
   *  see on screen still shows it happened. */
  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  function flash(message: string): void {
    toast = message;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 1600);
  }

  /** Copy the whole buffer — scrollback and screen — like PuTTY's "Copy All to
   *  Clipboard". Line endings follow the platform, as xterm's own selection does. */
  function copyAll(): void {
    if (!term) return;
    const text = bufferText(term.buffer.active, isWindows ? '\r\n' : '\n');
    if (!text) {
      flash($t('terminal.nothing_to_copy'));
      return;
    }
    const lines = text.split(isWindows ? '\r\n' : '\n').length;
    clipboardWriteText(text).then(
      () => flash($t('terminal.copied_lines', { count: lines })),
      (err) => lastError.set(`Copy failed: ${errText(err)}`)
    );
  }

  // ── The terminal menu: Ctrl+right-click (PuTTY), or every right-click when the
  //    setting says so.
  let menu = $state<{ x: number; y: number; hasSelection: boolean } | null>(null);

  function openMenu(e: MouseEvent): void {
    // Kept inside the window: the menu is ~11rem wide and ~10rem tall.
    const x = Math.min(e.clientX, window.innerWidth - 184);
    const y = Math.min(e.clientY, window.innerHeight - 168);
    menu = { x: Math.max(4, x), y: Math.max(4, y), hasSelection: !!term?.hasSelection() };
  }

  function closeMenu(): void {
    if (!menu) return;
    menu = null;
    if (get(dialogs).length === 0) term?.focus();
  }

  function menuAction(action: 'copy' | 'paste' | 'copyAll' | 'selectAll'): void {
    closeMenu();
    if (action === 'copy') copySelection();
    else if (action === 'paste') pasteFromClipboard();
    else if (action === 'copyAll') copyAll();
    else term?.selectAll();
  }

  // While the menu is open, Escape closes it and any press elsewhere dismisses it.
  function onMenuKey(e: KeyboardEvent): void {
    if (menu && e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeMenu();
    }
  }
  function onMenuOutside(e: MouseEvent): void {
    if (menu && !(e.target as Element | null)?.closest?.('[data-term-menu]')) menu = null;
  }

  /** A multi-line paste waiting on the user's say-so, when that check is on. */
  let pendingPaste = $state<string | null>(null);

  /** Hand `text` to xterm, which applies bracketed paste when the remote asked for it
   *  and turns line breaks into the CR a terminal sends for Enter. */
  function deliverPaste(text: string): void {
    if (!term || !text || ended()) return;
    term.paste(text);
  }

  function pasteText(text: string): void {
    if (!text || ended()) return;
    if (clipboardPrefs.get().confirmMultiLinePaste && isMultiLine(text)) {
      pendingPaste = text;
      return;
    }
    deliverPaste(text);
  }

  /** Paste the system clipboard. If the native read fails, fall back to the webview's
   *  own paste (WebKitGTK), which lands in xterm's textarea like a Ctrl+Shift+V. */
  function pasteFromClipboard(): void {
    if (ended()) return;
    clipboardReadText().then(pasteText, () =>
      terminalPaste().catch((err) => lastError.set(`Paste failed: ${errText(err)}`))
    );
  }

  function confirmPaste(): void {
    const text = pendingPaste;
    pendingPaste = null;
    if (text) deliverPaste(text);
  }

  /** The remote app is tracking the mouse (vim `mouse=a`, tmux, htop). */
  function mouseTracking(): boolean {
    return (term?.modes.mouseTrackingMode ?? 'none') !== 'none';
  }

  // Set once copy-on-select has copied the current selection, so the second release of
  // a double-click (or a click that leaves the selection be) does not copy it again.
  // Any change to the selection clears it.
  let autoCopied = false;

  // Mouse hooks on the host element. Registered once xterm has mounted into it.
  function onMouseUp(e: MouseEvent): void {
    // Copy on select: after a drag, a double-click word or a triple-click line.
    // Deferred a tick so xterm has settled the selection from this same mouseup.
    if (e.button !== 0 || destroyed || !clipboardPrefs.get().copyOnSelect) return;
    setTimeout(() => {
      if (!autoCopied && copySelection()) autoCopied = true;
    }, 0);
  }

  function onContextMenu(e: MouseEvent): void {
    // The webview's own menu (Reload / Inspect) never belongs over a terminal.
    e.preventDefault();
    const action = rightClickAction(
      clipboardPrefs.get().rightClick,
      !!term?.hasSelection(),
      mouseTracking(),
      e.shiftKey,
      e.ctrlKey
    );
    if (action === 'menu') {
      openMenu(e);
      return;
    }
    if (action === 'copy') {
      copySelection();
      term?.clearSelection();
    } else if (action === 'paste') {
      pasteFromClipboard();
    }
    term?.focus();
  }

  // Set while a middle-click is ours, so its release is swallowed too: Linux webviews
  // paste the PRIMARY selection on the release, which would paste a second time.
  let middleTaken = false;

  function onMiddleUp(e: MouseEvent): void {
    if (e.button !== 1 || !middleTaken) return;
    middleTaken = false;
    e.preventDefault();
    e.stopPropagation();
  }

  function onMouseDown(e: MouseEvent): void {
    if (e.button === 0) {
      // A drag can end outside the terminal (xterm keeps selecting past its edge), so
      // the release is watched on the window, once per press.
      window.addEventListener('mouseup', onMouseUp, { once: true });
      return;
    }
    if (e.button !== 1) return;
    if (!middleClickPastes(clipboardPrefs.get().middleClickPaste, mouseTracking(), e.shiftKey)) return;
    // Stops Windows' autoscroll puck and WebKitGTK's own PRIMARY paste (which would
    // paste a second time), and keeps the click from xterm.
    e.preventDefault();
    e.stopPropagation();
    middleTaken = true;
    pasteFromClipboard();
    term?.focus();
  }

  /** A paste the webview delivers itself (macOS Cmd+V from the Edit menu, or the
   *  fallback above) still gets the multi-line check. Capture phase, so this runs
   *  before xterm's own textarea handler. */
  function onNativePaste(e: ClipboardEvent): void {
    if (!clipboardPrefs.get().confirmMultiLinePaste) return;
    const text = e.clipboardData?.getData('text/plain') ?? '';
    if (!isMultiLine(text)) return;
    e.preventDefault();
    e.stopPropagation();
    pasteText(text);
  }

  /** Fit the terminal to its container and tell the backend, but only while visible —
   *  a hidden (display:none) container measures 0, so it refits when shown instead. */
  function safeFit(): void {
    if (!term || !fitAddon || !active) return;
    try {
      fitAddon.fit();
    } catch {
      return;
    }
    if (termId != null && !ended()) void terminalResize(termId, term.cols, term.rows).catch(() => {});
  }

  /** Open the find bar, or take it back to the query if it is already open. A one-line
   *  selection becomes the query, as in most editors. */
  function openFind(): void {
    const seed = seedQuery(term?.getSelection() ?? '');
    if (findOpen) {
      findBar?.focus(seed);
    } else {
      findSeed = seed;
      findOpen = true;
    }
  }

  function closeFind(): void {
    findOpen = false;
    if (keyboardFree()) term?.focus();
  }

  function scheduleFit(): void {
    if (fitScheduled) return;
    fitScheduled = true;
    requestAnimationFrame(() => {
      fitScheduled = false;
      safeFit();
    });
  }

  onMount(() => {
    void (async () => {
      const [{ Terminal }, { FitAddon }, { SearchAddon }, { WebLinksAddon }] = await Promise.all([
        import('@xterm/xterm'),
        import('@xterm/addon-fit'),
        import('@xterm/addon-search'),
        import('@xterm/addon-web-links')
      ]);
      if (destroyed) return;

      const initPrefs = terminalPrefs.get();
      term = new Terminal({
        fontFamily: resolveTerminalFontStack(fontPrefs.get(), MONO),
        fontSize: initPrefs.fontSize,
        lineHeight: initPrefs.lineHeight,
        cursorStyle: initPrefs.cursorStyle,
        cursorBlink: initPrefs.cursorBlink,
        scrollback: initPrefs.scrollback,
        // The search addon highlights matches through registerDecoration, which xterm
        // still gates as proposed API.
        allowProposedApi: true,
        // Right-click is ours (paste / copy-or-paste), so it must not first replace
        // the selection with the word under the pointer, as xterm does on macOS.
        rightClickSelectsWord: false
      });
      fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      const search = new SearchAddon({ highlightLimit: HIGHLIGHT_LIMIT });
      term.loadAddon(search);
      searchAddon = search;
      term.loadAddon(
        new WebLinksAddon((_event, uri) => {
          void openExternal(uri);
        })
      );
      term.open(container);
      term.onScroll(syncScrolled);
      term.onSelectionChange(() => (autoCopied = false));
      container.addEventListener('contextmenu', onContextMenu);
      container.addEventListener('mousedown', onMouseDown, true);
      container.addEventListener('mouseup', onMiddleUp, true);
      container.addEventListener('auxclick', onMiddleUp, true);
      container.addEventListener('paste', onNativePaste, true);
      container.addEventListener('wheel', onWheel, { passive: false });
      window.addEventListener('keydown', onMenuKey, true);
      window.addEventListener('mousedown', onMenuOutside, true);

      // Keep xterm theme, font, zoom, cursor, scrollback, and keyword highlights live-synced.
      themeUnsub = theme.subscribe((appTheme) => {
        if (term) term.options.theme = xtermTheme(appTheme, terminalPrefs.get().colorScheme);
      });
      let firstFontSync = true;
      fontUnsub = fontPrefs.subscribe((prefs) => {
        if (!term) return;
        const nextFamily = resolveTerminalFontStack(prefs, MONO);
        if (term.options.fontFamily !== nextFamily) {
          term.options.fontFamily = nextFamily;
          if (!firstFontSync) scheduleFit();
        }
        firstFontSync = false;
      });
      let firstPrefsSync = true;
      prefsUnsub = terminalPrefs.subscribe((prefs) => {
        highlightActive = prefs.highlightEnabled;
        compiledHighlights = compileHighlightRules(prefs.highlightRules);
        if (!term) return;
        const needRefit =
          term.options.fontSize !== prefs.fontSize || term.options.lineHeight !== prefs.lineHeight;
        term.options.fontSize = prefs.fontSize;
        term.options.lineHeight = prefs.lineHeight;
        term.options.scrollback = prefs.scrollback;
        term.options.cursorStyle = prefs.cursorStyle;
        term.options.cursorBlink = prefs.cursorBlink;
        term.options.theme = xtermTheme(get(theme), prefs.colorScheme);
        if (needRefit && !firstPrefsSync) scheduleFit();
        firstPrefsSync = false;
      });

      // Fit before opening so the PTY starts at the visible size.
      safeFit();
      await openBackendPty();
      if (destroyed) return;

      // Lazy-load snippets for the quick command bar if the store is empty.
      if (get(snippets).length === 0 && typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
        listSnippets()
          .then((list) => snippets.set(list))
          .catch(() => {});
      }

      // Copy takes Ctrl+Shift+C whether or not anything is selected, so the chord never
      // reaches the shell. Returning false only keeps xterm out of it; the default is
      // ours to stop. The write happens inside the keydown, which WebKit requires.
      term.attachCustomKeyEventHandler((e) => {
        // Zoom shortcuts: Ctrl/Cmd + = / - / 0.
        if (e.type === 'keydown' && (e.ctrlKey || e.metaKey) && !e.altKey) {
          if (e.key === '=' || e.key === '+') {
            e.preventDefault();
            terminalPrefs.zoom(1);
            return false;
          }
          if (e.key === '-') {
            e.preventDefault();
            terminalPrefs.zoom(-1);
            return false;
          }
          if (e.key === '0') {
            e.preventDefault();
            terminalPrefs.resetZoom();
            return false;
          }
        }
        // The streamer chord belongs to the window (AppShell), never to the shell, where
        // a ^S would stop its output.
        if (isStreamerChord(e)) return false;
        // Find works on an ended session too: its last output is what is left to read.
        if (isFindShortcut(e, isMac)) {
          e.preventDefault();
          openFind();
          return false;
        }
        if (isCopyAllShortcut(e, isMac)) {
          e.preventDefault();
          copyAll();
          return false;
        }
        if (isCopyShortcut(e, isMac) || isCopyInsert(e)) {
          e.preventDefault();
          copySelection();
          return false;
        }
        // Shift+Insert, and Ctrl+Shift+V off macOS: paste from the native clipboard
        // instead of the webview's, so every paste takes the same path.
        if (isPasteShortcut(e, isMac)) {
          e.preventDefault();
          e.stopPropagation();
          pasteFromClipboard();
          return false;
        }
        // An ended session takes no input: Enter or Esc closes the tab, and xterm gets
        // no other key (`sendInput` drops anything that still gets through).
        if (ended()) {
          if (closesEndedTab(e)) {
            e.preventDefault();
            closeSession(session.id);
          } else if (e.type === 'keydown' && e.key === 'Tab') {
            // xterm no longer takes Tab here, and moving focus off the terminal
            // would hand the Enter that closes the tab to a button.
            e.preventDefault();
          }
          return false;
        }
        // Under a non-Latin layout WebKitGTK names no key; the physical one stands in.
        const fallback = layoutFallback(e);
        if (!fallback) return true;
        // As xterm does with a key it handles: nothing else acts on it.
        e.preventDefault();
        e.stopPropagation();
        if (fallback.kind === 'control') {
          term?.input(fallback.data);
        } else {
          pasteFromClipboard();
        }
        return false;
      });
      // Text keystrokes/paste are UTF-8; onBinary carries raw 8-bit sequences
      // (e.g. legacy mouse reporting) that must go byte-for-byte, not re-encoded.
      term.onData((data) => sendInput(ENCODER.encode(data)));
      term.onBinary((data) => sendInput(Uint8Array.from(data, (ch) => ch.charCodeAt(0) & 0xff)));

      resizeObserver = new ResizeObserver(() => scheduleFit());
      resizeObserver.observe(container);

      ready = true;
      if (active && keyboardFree()) term.focus();
    })().catch((err) => {
      // `terminal_open` itself failed (e.g. the session could not be spawned): no
      // PtyExited follows, so mark the tab failed here instead of leaving it hung.
      lastError.set(err instanceof Error ? err.message : String(err));
      sessions.setStatus(session.id, 'failed');
    });
  });

  onDestroy(() => {
    destroyed = true;
    themeUnsub?.();
    fontUnsub?.();
    prefsUnsub?.();
    resizeObserver?.disconnect();
    window.removeEventListener('mouseup', onMouseUp);
    container?.removeEventListener('contextmenu', onContextMenu);
    container?.removeEventListener('mousedown', onMouseDown, true);
    container?.removeEventListener('mouseup', onMiddleUp, true);
    container?.removeEventListener('auxclick', onMiddleUp, true);
    container?.removeEventListener('paste', onNativePaste, true);
    container?.removeEventListener('wheel', onWheel);
    window.removeEventListener('keydown', onMenuKey, true);
    window.removeEventListener('mousedown', onMenuOutside, true);
    clearTimeout(toastTimer);
    // Idempotent: a remote-exit teardown already dropped this id backend-side (§3.4).
    if (termId != null) void terminalClose(termId).catch(() => {});
    term?.dispose();
    // Null it so a byte still in flight (destroyed-before-open race) can't write to
    // a disposed terminal — the channel callback's `if (!term)` guard then bails.
    term = undefined;
  });

  // Becoming visible: a hidden container measured 0, so refit and take focus. Also
  // when a dialog or the palette closes, and when the active tab is clicked again,
  // which moves focus to the tab without changing `active`. The keyboard check runs
  // in the frame, since a dialog can open before it.
  $effect(() => {
    if (!active) menu = null;
  });

  $effect(() => {
    if (active && ready) {
      void [$dialogs, $palette, $activeEntity, sftpDrawerOpen, quickBarOpen, paneClass];
      requestAnimationFrame(() => {
        safeFit();
        // An open find bar keeps the keyboard it had when the tab was left.
        if (focused && keyboardFree()) {
          if (findOpen) findBar?.focus();
          else term?.focus();
        }
        syncScrolled();
      });
    }
  });

  const tbBtn =
    'inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-[11px] text-muted transition ' +
    'hover:bg-surface-inset hover:text-fg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus';
</script>

<!-- bg-surface fills behind the macOS traffic lights (no seam). Text selection stays
     disabled app-wide (app.css); the terminal is the one selectable surface, handled
     by xterm's own selection (not CSS). -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="absolute {paneClass} flex flex-col overflow-hidden bg-surface {active ? '' : 'hidden'} {isSplit && focused
    ? 'ring-1 ring-inset ring-accent'
    : ''}"
  style="padding-top: var(--titlebar-h);"
  onmousedown={() => onPaneFocus?.()}
>
  <!-- Compact terminal action bar -->
  <div class="flex h-8 shrink-0 items-center justify-between gap-2 border-b border-default bg-surface px-2.5">
    <div class="flex min-w-0 items-center gap-2">
      <span class="truncate text-xs font-medium text-fg">
        {session.isLocal ? $t('sidebar.local_shell') : displayHostTitle(session.hostName, $streamerMode)}
      </span>
      {#if recording}
        <span class="inline-flex items-center gap-1 rounded-full bg-surface-inset px-2 py-0.5 text-[10px] font-semibold text-status-crit">
          <span class="h-1.5 w-1.5 rounded-full bg-status-crit"></span>
          {$t('terminal.recording_badge')}
        </span>
      {/if}
      {#if sessionEnded}
        <button
          type="button"
          class="inline-flex items-center gap-1 rounded-full bg-accent px-2.5 py-0.5 text-[11px] font-medium text-accent-fg transition hover:opacity-90"
          disabled={reconnecting}
          onclick={() => void reconnect()}
        >
          <Icon name="refresh" size={11} />
          {$t('terminal.reconnect')}
        </button>
      {/if}
    </div>
    <div class="flex shrink-0 items-center gap-0.5">
      <button
        type="button"
        class={tbBtn}
        title={$t('terminal.reconnect')}
        aria-label={$t('terminal.reconnect')}
        disabled={reconnecting}
        onclick={() => void reconnect()}
      >
        <Icon name="refresh" size={13} />
      </button>
      <button
        type="button"
        class={tbBtn}
        title={$t('terminal.duplicate')}
        aria-label={$t('terminal.duplicate')}
        onclick={() => duplicateSession(session)}
      >
        <Icon name="copy" size={13} />
      </button>
      {#if isSplit}
        <button
          type="button"
          class={tbBtn}
          title={$t('terminal.unsplit')}
          aria-label={$t('terminal.unsplit')}
          onclick={() => splitLayout.unsplit()}
        >
          <Icon name="close" size={13} />
        </button>
      {:else}
        <button
          type="button"
          class={tbBtn}
          title={$t('terminal.split_horizontal')}
          aria-label={$t('terminal.split_horizontal')}
          onclick={() => splitWithDuplicate(session, 'horizontal')}
        >
          <Icon name="splitH" size={13} />
        </button>
        <button
          type="button"
          class={tbBtn}
          title={$t('terminal.split_vertical')}
          aria-label={$t('terminal.split_vertical')}
          onclick={() => splitWithDuplicate(session, 'vertical')}
        >
          <Icon name="splitV" size={13} />
        </button>
      {/if}
      <button
        type="button"
        class="{tbBtn} {recording ? 'text-status-crit' : ''}"
        title={recording ? $t('terminal.record_stop') : $t('terminal.record_start')}
        aria-label={recording ? $t('terminal.record_stop') : $t('terminal.record_start')}
        aria-pressed={recording}
        onclick={() => void toggleRecording()}
      >
        <Icon name="record" size={13} />
      </button>
      <button
        type="button"
        class="{tbBtn} {highlightActive ? 'bg-surface-inset text-fg' : ''}"
        title={$t('terminal.highlights_toggle')}
        aria-label={$t('terminal.highlights_toggle')}
        aria-pressed={highlightActive}
        onclick={() => terminalPrefs.update({ highlightEnabled: !highlightActive })}
      >
        <Icon name="sparkles" size={13} />
      </button>
      <button
        type="button"
        class={tbBtn}
        title={$t('terminal.zoom_out')}
        aria-label={$t('terminal.zoom_out')}
        onclick={() => terminalPrefs.zoom(-1)}
      >
        <Icon name="zoomOut" size={13} />
      </button>
      <button
        type="button"
        class="{tbBtn} tabular-nums"
        title={$t('terminal.zoom_reset', { size: $terminalPrefs.fontSize })}
        onclick={() => terminalPrefs.resetZoom()}
      >
        {$terminalPrefs.fontSize}px
      </button>
      <button
        type="button"
        class={tbBtn}
        title={$t('terminal.zoom_in')}
        aria-label={$t('terminal.zoom_in')}
        onclick={() => terminalPrefs.zoom(1)}
      >
        <Icon name="zoomIn" size={13} />
      </button>
      <button
        type="button"
        class="{tbBtn} {quickBarOpen ? 'bg-surface-inset text-fg' : ''}"
        title={$t('terminal.quick_commands')}
        aria-label={$t('terminal.quick_commands')}
        aria-pressed={quickBarOpen}
        onclick={() => (quickBarOpen = !quickBarOpen)}
      >
        <Icon name="snippets" size={13} />
      </button>
      {#if !session.isLocal}
        <button
          type="button"
          class="{tbBtn} {sftpDrawerOpen ? 'bg-surface-inset text-fg' : ''}"
          title={$t('terminal.sftp_sidebar')}
          aria-label={$t('terminal.sftp_sidebar')}
          aria-pressed={sftpDrawerOpen}
          onclick={() => (sftpDrawerOpen = !sftpDrawerOpen)}
        >
          <Icon name="sidebar" size={13} />
        </button>
      {/if}
      <button
        type="button"
        class={tbBtn}
        title={$t('terminal.find')}
        aria-label={$t('terminal.find')}
        onclick={openFind}
      >
        <Icon name="search" size={13} />
      </button>
    </div>
  </div>

  <!-- Terminal canvas + optional right-hand SFTP sidebar -->
  <div class="relative flex min-h-0 flex-1 overflow-hidden">
    <div class="relative min-h-0 min-w-0 flex-1" style="padding: 0.4rem 0.5rem 0.75rem;">
      <div bind:this={container} class="h-full w-full" class:term-fade={scrolled}></div>
      {#if findOpen && searchAddon}
        <TerminalFind bind:this={findBar} search={searchAddon} seed={findSeed} onclose={closeFind} />
      {/if}
      {#if toast}
        <div
          role="status"
          class="pointer-events-none absolute bottom-4 right-4 rounded-full bg-surface-raised px-3 py-1.5 text-xs text-fg shadow-soft"
        >
          {toast}
        </div>
      {/if}
    </div>
    {#if sftpDrawerOpen && !session.isLocal}
      <TerminalSftpDrawer hostName={session.hostName} onClose={() => (sftpDrawerOpen = false)} />
    {/if}
  </div>

  <!-- Bottom collapsible quick command bar -->
  {#if quickBarOpen}
    <div
      data-quick-commands
      class="flex shrink-0 items-center gap-2 border-t border-default bg-surface-raised px-2.5 py-1.5"
    >
      <input
        bind:value={quickFilter}
        type="text"
        placeholder={$t('terminal.quick_commands_filter')}
        aria-label={$t('terminal.quick_commands_filter')}
        class="w-36 shrink-0 rounded-md border border-default bg-surface-inset px-2 py-1 text-xs text-fg placeholder:text-faint focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-focus"
      />
      <div class="flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto">
        {#if quickSnippets.length === 0}
          <span class="truncate text-xs text-faint">{$t('terminal.quick_commands_empty')}</span>
        {:else}
          {#each quickSnippets as sn (sn.name)}
            <button
              type="button"
              class="shrink-0 rounded-full border border-default bg-surface px-2.5 py-0.5 text-xs text-fg transition hover:border-strong hover:bg-accent hover:text-accent-fg"
              title={sn.command}
              onclick={() => sendQuickSnippet(sn)}
            >
              {sn.name}
            </button>
          {/each}
        {/if}
      </div>
    </div>
  {/if}
</div>

{#if paramPrompt}
  <Modal label={paramPrompt.snippet.name} onClose={() => (paramPrompt = null)}>
    <form
      class="space-y-3 p-5"
      onsubmit={(e) => {
        e.preventDefault();
        submitParamSnippet();
      }}
    >
      <h2 class="text-sm font-semibold">{paramPrompt.snippet.name}</h2>
      <p class="font-mono text-xs text-muted">{paramPrompt.snippet.command}</p>
      {#each Object.keys(paramPrompt.values) as key (key)}
        <div>
          <label class="mb-1 block text-xs text-muted" for="qc-param-{key}">{key}</label>
          <input
            id="qc-param-{key}"
            bind:value={paramPrompt.values[key]}
            type="text"
            class="w-full rounded-xl border border-default bg-surface-inset px-3 py-1.5 text-sm text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
          />
        </div>
      {/each}
      <div class="flex justify-end gap-2 pt-2">
        <button
          type="button"
          class="rounded-full px-4 py-1.5 text-sm text-muted hover:bg-surface-inset hover:text-fg"
          onclick={() => (paramPrompt = null)}
        >
          {$t('common.cancel')}
        </button>
        <button
          type="submit"
          class="rounded-full bg-accent px-4 py-1.5 text-sm font-medium text-accent-fg"
        >
          {$t('common.confirm')}
        </button>
      </div>
    </form>
  </Modal>
{/if}

{#if menu}
  <div
    data-term-menu
    role="menu"
    aria-label="Terminal"
    class="fixed z-40 w-44 overflow-hidden rounded-xl border border-default bg-surface-raised py-1 text-sm shadow-soft"
    style="left: {menu.x}px; top: {menu.y}px;"
  >
    {@render menuItem($t('terminal.menu_copy'), isMac ? '⌘C' : 'Ctrl+Shift+C', menu.hasSelection, () => menuAction('copy'))}
    {@render menuItem($t('terminal.menu_paste'), isMac ? '⌘V' : 'Ctrl+Shift+V', true, () => menuAction('paste'))}
    <div class="my-1 border-t border-default"></div>
    {@render menuItem($t('terminal.menu_copy_all'), isMac ? '⇧⌘A' : 'Ctrl+Shift+A', true, () => menuAction('copyAll'))}
    {@render menuItem($t('terminal.menu_select_all'), '', true, () => menuAction('selectAll'))}
  </div>
{/if}

{#snippet menuItem(label: string, keys: string, enabled: boolean, run: () => void)}
  <button
    type="button"
    role="menuitem"
    disabled={!enabled}
    onclick={run}
    class="flex w-full items-center justify-between gap-3 px-3 py-1.5 text-left transition hover:bg-accent hover:text-accent-fg disabled:pointer-events-none disabled:opacity-40"
  >
    <span>{label}</span>
    {#if keys}<span class="text-xs opacity-60">{keys}</span>{/if}
  </button>
{/snippet}

{#if pendingPaste !== null}
  <Modal label={$t('terminal.paste_confirm_label')} onClose={() => (pendingPaste = null)}>
    <div class="p-5">
      <h2 class="text-sm font-semibold">{$t('terminal.paste_confirm_title', { count: lineCount(pendingPaste) })}</h2>
      <p class="mt-1 text-xs text-muted">
        {$t('terminal.paste_confirm_desc')}
      </p>
      <pre
        class="mt-3 max-h-48 overflow-auto rounded-lg bg-surface-inset p-3 font-mono text-xs leading-relaxed">{pendingPaste.length > 4000
          ? pendingPaste.slice(0, 4000) + '\n…'
          : pendingPaste}</pre>
      <div class="mt-4 flex justify-end gap-2">
        <button
          type="button"
          class="rounded-full px-4 py-1.5 text-sm text-muted transition hover:bg-surface-inset hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
          onclick={() => (pendingPaste = null)}
        >
          {$t('common.cancel')}
        </button>
        <!-- svelte-ignore a11y_autofocus -->
        <button
          type="button"
          autofocus
          class="rounded-full bg-accent px-4 py-1.5 text-sm text-accent-fg transition hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
          onclick={confirmPaste}
        >
          {$t('terminal.menu_paste')}
        </button>
      </div>
    </div>
  </Modal>
{/if}

<style>
  /* Scrolled output dissolves into the top edge instead of hard-clipping (on only while
     scrolled, so the first line stays crisp). black/transparent are mask alphas. */
  .term-fade {
    -webkit-mask-image: linear-gradient(to bottom, transparent, black 2.25rem);
    mask-image: linear-gradient(to bottom, transparent, black 2.25rem);
  }
</style>
