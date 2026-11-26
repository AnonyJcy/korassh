import { writable } from 'svelte/store';

// Terminal appearance, behavior, and keyword-highlight preferences.
// Persisted via tauri-plugin-store (`settings.json`) with a localStorage mirror.

export type CursorStyle = 'block' | 'underline' | 'bar';

export type TerminalColorScheme =
  | 'auto'
  | 'dracula'
  | 'nord'
  | 'oneDark'
  | 'tokyoNight'
  | 'gruvboxDark'
  | 'solarizedDark'
  | 'solarizedLight';

export type HighlightColor = 'red' | 'yellow' | 'green' | 'cyan' | 'magenta' | 'blue';

export interface HighlightRule {
  id: string;
  label: string;
  pattern: string;
  color: HighlightColor;
  enabled: boolean;
}

export interface TerminalPrefs {
  fontSize: number;
  lineHeight: number;
  scrollback: number;
  cursorStyle: CursorStyle;
  cursorBlink: boolean;
  colorScheme: TerminalColorScheme;
  highlightEnabled: boolean;
  highlightRules: HighlightRule[];
}

export const TERMINAL_COLOR_SCHEMES: readonly TerminalColorScheme[] = [
  'auto',
  'dracula',
  'nord',
  'oneDark',
  'tokyoNight',
  'gruvboxDark',
  'solarizedDark',
  'solarizedLight'
];

export const CURSOR_STYLES: readonly CursorStyle[] = ['block', 'underline', 'bar'];
export const SCROLLBACK_OPTIONS: readonly number[] = [2000, 5000, 10000, 25000, 50000];
export const HIGHLIGHT_COLORS: readonly HighlightColor[] = ['red', 'yellow', 'green', 'cyan', 'magenta', 'blue'];

export const DEFAULT_HIGHLIGHT_RULES: HighlightRule[] = [
  { id: 'err', label: 'Error', pattern: '\\b(ERROR|FATAL|PANIC|CRITICAL|FAILED|Exception)\\b', color: 'red', enabled: true },
  { id: 'warn', label: 'Warn', pattern: '\\b(WARN|WARNING|DEPRECATED)\\b', color: 'yellow', enabled: true },
  { id: 'ok', label: 'Success', pattern: '\\b(SUCCESS|PASSED|CONNECTED|READY)\\b', color: 'green', enabled: true },
  { id: 'info', label: 'Info', pattern: '\\b(INFO|NOTICE)\\b', color: 'cyan', enabled: false }
];

export const DEFAULT_TERMINAL_PREFS: TerminalPrefs = {
  fontSize: 13,
  lineHeight: 1.15,
  scrollback: 10000,
  cursorStyle: 'block',
  cursorBlink: true,
  colorScheme: 'auto',
  highlightEnabled: true,
  highlightRules: DEFAULT_HIGHLIGHT_RULES
};

export const MIN_FONT_SIZE = 10;
export const MAX_FONT_SIZE = 24;

const LOCAL_KEY = 'korassh-terminal-prefs';
const STORE_FILE = 'settings.json';
const STORE_KEY = 'terminalPrefs';

export function clampFontSize(v: unknown): number {
  const n = typeof v === 'number' ? v : Number(v);
  return Number.isFinite(n) ? Math.min(MAX_FONT_SIZE, Math.max(MIN_FONT_SIZE, Math.round(n))) : DEFAULT_TERMINAL_PREFS.fontSize;
}

export function clampLineHeight(v: unknown): number {
  const n = typeof v === 'number' ? v : Number(v);
  if (!Number.isFinite(n)) return DEFAULT_TERMINAL_PREFS.lineHeight;
  return Math.round(Math.min(1.8, Math.max(1.0, n)) * 100) / 100;
}

export function clampScrollback(v: unknown): number {
  const n = typeof v === 'number' ? v : Number(v);
  return Number.isFinite(n) ? Math.min(50000, Math.max(1000, Math.round(n))) : DEFAULT_TERMINAL_PREFS.scrollback;
}

function isCursorStyle(v: unknown): v is CursorStyle {
  return v === 'block' || v === 'underline' || v === 'bar';
}

function isColorScheme(v: unknown): v is TerminalColorScheme {
  return typeof v === 'string' && (TERMINAL_COLOR_SCHEMES as readonly string[]).includes(v);
}

function isHighlightColor(v: unknown): v is HighlightColor {
  return typeof v === 'string' && (HIGHLIGHT_COLORS as readonly string[]).includes(v);
}

function parseRules(raw: unknown): HighlightRule[] {
  if (!Array.isArray(raw)) return DEFAULT_HIGHLIGHT_RULES.map((r) => ({ ...r }));
  const parsed: HighlightRule[] = [];
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue;
    const r = item as Record<string, unknown>;
    if (typeof r.id !== 'string' || typeof r.pattern !== 'string') continue;
    parsed.push({
      id: r.id.slice(0, 40),
      label: typeof r.label === 'string' ? r.label.slice(0, 40) : r.pattern.slice(0, 20),
      pattern: r.pattern.slice(0, 200),
      color: isHighlightColor(r.color) ? r.color : 'yellow',
      enabled: typeof r.enabled === 'boolean' ? r.enabled : true
    });
  }
  return parsed.length > 0 ? parsed : DEFAULT_HIGHLIGHT_RULES.map((r) => ({ ...r }));
}

export function parseTerminalPrefs(raw: unknown): TerminalPrefs {
  if (!raw || typeof raw !== 'object') {
    return {
      ...DEFAULT_TERMINAL_PREFS,
      highlightRules: DEFAULT_HIGHLIGHT_RULES.map((r) => ({ ...r }))
    };
  }
  const r = raw as Record<string, unknown>;
  return {
    fontSize: clampFontSize(r.fontSize),
    lineHeight: clampLineHeight(r.lineHeight),
    scrollback: clampScrollback(r.scrollback),
    cursorStyle: isCursorStyle(r.cursorStyle) ? r.cursorStyle : DEFAULT_TERMINAL_PREFS.cursorStyle,
    cursorBlink: typeof r.cursorBlink === 'boolean' ? r.cursorBlink : DEFAULT_TERMINAL_PREFS.cursorBlink,
    colorScheme: isColorScheme(r.colorScheme) ? r.colorScheme : DEFAULT_TERMINAL_PREFS.colorScheme,
    highlightEnabled:
      typeof r.highlightEnabled === 'boolean' ? r.highlightEnabled : DEFAULT_TERMINAL_PREFS.highlightEnabled,
    highlightRules: parseRules(r.highlightRules)
  };
}

function readMirror(): TerminalPrefs {
  try {
    const raw = localStorage.getItem(LOCAL_KEY);
    return raw ? parseTerminalPrefs(JSON.parse(raw)) : parseTerminalPrefs(null);
  } catch {
    return parseTerminalPrefs(null);
  }
}

function writeMirror(prefs: TerminalPrefs): void {
  try {
    localStorage.setItem(LOCAL_KEY, JSON.stringify(prefs));
  } catch {}
}

async function persistStore(prefs: TerminalPrefs): Promise<void> {
  try {
    const { load } = await import('@tauri-apps/plugin-store');
    const store = await load(STORE_FILE);
    await store.set(STORE_KEY, prefs);
    await store.save();
  } catch {}
}

function createTerminalPrefs() {
  const initial = readMirror();
  const { subscribe, set: setStore } = writable<TerminalPrefs>(initial);
  let current = initial;
  let interacted = false;

  function apply(next: TerminalPrefs, user: boolean): void {
    current = next;
    setStore(next);
    writeMirror(next);
    if (user) {
      interacted = true;
      void persistStore(next);
    }
  }

  return {
    subscribe,
    get: (): TerminalPrefs => current,
    update(patch: Partial<TerminalPrefs>): void {
      apply(parseTerminalPrefs({ ...current, ...patch }), true);
    },
    zoom(delta: number): void {
      apply(parseTerminalPrefs({ ...current, fontSize: current.fontSize + delta }), true);
    },
    resetZoom(): void {
      apply(parseTerminalPrefs({ ...current, fontSize: DEFAULT_TERMINAL_PREFS.fontSize }), true);
    },
    reset(): void {
      apply(parseTerminalPrefs(null), true);
    },
    async hydrate(): Promise<void> {
      try {
        const { load } = await import('@tauri-apps/plugin-store');
        const store = await load(STORE_FILE);
        const saved = await store.get<unknown>(STORE_KEY);
        if (!interacted && saved != null) apply(parseTerminalPrefs(saved), false);
      } catch {}
    }
  };
}

export const terminalPrefs = createTerminalPrefs();
