// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import {
  DEFAULT_TERMINAL_PREFS,
  MAX_FONT_SIZE,
  MIN_FONT_SIZE,
  clampFontSize,
  clampLineHeight,
  clampScrollback,
  parseTerminalPrefs,
  terminalPrefs
} from './terminalPrefs';

describe('terminal preferences', () => {
  beforeEach(() => {
    localStorage.clear();
    terminalPrefs.reset();
  });

  it('clamps font size, line height, and scrollback cleanly', () => {
    expect(clampFontSize(4)).toBe(MIN_FONT_SIZE);
    expect(clampFontSize(99)).toBe(MAX_FONT_SIZE);
    expect(clampLineHeight(0.5)).toBe(1.0);
    expect(clampLineHeight(2.5)).toBe(1.8);
    expect(clampScrollback(10)).toBe(1000);
    expect(clampScrollback(999999)).toBe(50000);
  });

  it('zooms font size incrementally and resets cleanly', () => {
    expect(terminalPrefs.get().fontSize).toBe(DEFAULT_TERMINAL_PREFS.fontSize);
    terminalPrefs.zoom(2);
    expect(terminalPrefs.get().fontSize).toBe(15);
    terminalPrefs.zoom(-3);
    expect(terminalPrefs.get().fontSize).toBe(12);
    terminalPrefs.resetZoom();
    expect(terminalPrefs.get().fontSize).toBe(DEFAULT_TERMINAL_PREFS.fontSize);
  });

  it('updates color schemes, cursor styles, and ignores malformed stored blobs', () => {
    terminalPrefs.update({ colorScheme: 'dracula', cursorStyle: 'bar', cursorBlink: false });
    expect(terminalPrefs.get()).toMatchObject({
      colorScheme: 'dracula',
      cursorStyle: 'bar',
      cursorBlink: false
    });
    expect(parseTerminalPrefs({ colorScheme: 'bogus', cursorStyle: 123 })).toMatchObject({
      colorScheme: 'auto',
      cursorStyle: 'block'
    });
  });
});
