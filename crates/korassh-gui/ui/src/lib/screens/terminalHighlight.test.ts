import { describe, expect, it } from 'vitest';
import { applyKeywordHighlights, compileHighlightRules, stripAnsi } from './terminalHighlight';
import { DEFAULT_HIGHLIGHT_RULES } from '$lib/stores/terminalPrefs';

describe('terminal keyword highlighting and log stripping', () => {
  it('highlights matching keywords outside ANSI sequences without corrupting escapes', () => {
    const compiled = compileHighlightRules(DEFAULT_HIGHLIGHT_RULES);
    const input = '\x1b[34m[service]\x1b[0m ERROR connection failed, WARN retrying';
    const out = applyKeywordHighlights(input, compiled);
    expect(out).toContain('\x1b[34m[service]\x1b[0m');
    expect(out).toContain('\x1b[1;31mERROR\x1b[22;39m');
    expect(out).toContain('\x1b[1;33mWARN\x1b[22;39m');
    expect(stripAnsi(out)).toBe('[service] ERROR connection failed, WARN retrying');
  });

  it('skips invalid regex patterns gracefully', () => {
    const compiled = compileHighlightRules([
      { id: 'bad', label: 'Bad', pattern: '[unclosed', color: 'red', enabled: true },
      { id: 'good', label: 'Good', pattern: 'READY', color: 'green', enabled: true }
    ]);
    expect(compiled).toHaveLength(1);
    expect(applyKeywordHighlights('server READY', compiled)).toContain('\x1b[1;32mREADY\x1b[22;39m');
  });

  it('strips CSI and OSC sequences for clean log recording', () => {
    expect(stripAnsi('\x1b]0;title\x07\x1b[1;32mhello\x1b[0m\r\nworld\r')).toBe('hello\nworld');
  });
});
