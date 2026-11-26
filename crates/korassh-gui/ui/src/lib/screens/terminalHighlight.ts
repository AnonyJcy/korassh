import type { HighlightColor, HighlightRule } from '$lib/stores/terminalPrefs';

// ANSI escape sequence tokenizer: CSI (\x1b[...), OSC (\x1b]...\x07 or \x1b\\), and 2-byte escapes.
const ANSI_TOKEN_RE = /(\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_])/g;

const ANSI_COLOR_CODE: Record<HighlightColor, string> = {
  red: '\x1b[1;31m',
  yellow: '\x1b[1;33m',
  green: '\x1b[1;32m',
  cyan: '\x1b[1;36m',
  magenta: '\x1b[1;35m',
  blue: '\x1b[1;34m'
};

const ANSI_RESET_FG = '\x1b[22;39m';

export interface CompiledHighlightRule {
  regex: RegExp;
  code: string;
}

/** Compile active highlight rules, silently skipping invalid regexes so a half-typed
 *  custom rule never disrupts live terminal rendering. */
export function compileHighlightRules(rules: HighlightRule[]): CompiledHighlightRule[] {
  const out: CompiledHighlightRule[] = [];
  for (const rule of rules) {
    if (!rule.enabled || !rule.pattern.trim()) continue;
    try {
      out.push({
        regex: new RegExp(rule.pattern, 'g'),
        code: ANSI_COLOR_CODE[rule.color] ?? ANSI_COLOR_CODE.yellow
      });
    } catch {
      // Ignore invalid regex while editing.
    }
  }
  return out;
}

/** Inject ANSI highlight codes into plain text segments outside existing ANSI sequences. */
export function applyKeywordHighlights(text: string, compiled: CompiledHighlightRule[]): string {
  if (compiled.length === 0 || text.length === 0) return text;
  const parts = text.split(ANSI_TOKEN_RE);
  for (let i = 0; i < parts.length; i += 2) {
    let seg = parts[i];
    if (!seg) continue;
    for (const rule of compiled) {
      rule.regex.lastIndex = 0;
      seg = seg.replace(rule.regex, (m) => (m ? `${rule.code}${m}${ANSI_RESET_FG}` : m));
    }
    parts[i] = seg;
  }
  return parts.join('');
}

/** Strip ANSI escape sequences and carriage-return control noise for clean `.log` exports. */
export function stripAnsi(text: string): string {
  return text.replace(ANSI_TOKEN_RE, '').replace(/\r\n/g, '\n').replace(/\r/g, '');
}
