// xterm renders outside the DOM/CSS-variable system, so its colours are declared here
// and pushed live to every open terminal whenever the app theme or chosen terminal color
// scheme changes. Supports `auto` (mirrors app ink/paper light & dark tokens) plus 7
// classic curated palettes.
import type { ITheme } from '@xterm/xterm';
import type { Theme } from '$lib/stores/theme';
import type { TerminalColorScheme } from '$lib/stores/terminalPrefs';

const dark: ITheme = {
  background: '#212121',
  foreground: '#e4e7ec',
  cursor: '#ffffff',
  cursorAccent: '#212121',
  selectionBackground: 'rgba(255, 255, 255, 0.18)',
  black: '#3a3f45',
  red: '#f07777',
  green: '#5fd08a',
  yellow: '#e8c15a',
  blue: '#7aa2f7',
  magenta: '#bb9af7',
  cyan: '#7dcfff',
  white: '#c8ccd4',
  brightBlack: '#5b6066',
  brightRed: '#ff8f8f',
  brightGreen: '#7fe0a3',
  brightYellow: '#f0d178',
  brightBlue: '#9ab8ff',
  brightMagenta: '#d0b8ff',
  brightCyan: '#9ee0ff',
  brightWhite: '#ffffff'
};

const light: ITheme = {
  background: '#ffffff',
  foreground: '#212121',
  cursor: '#212121',
  cursorAccent: '#ffffff',
  selectionBackground: 'rgba(33, 33, 33, 0.15)',
  black: '#212121',
  red: '#c0392b',
  green: '#1e824c',
  yellow: '#b7791f',
  blue: '#2f6feb',
  magenta: '#8250df',
  cyan: '#1b7c83',
  white: '#aeb7c2',
  brightBlack: '#5b6066',
  brightRed: '#d64545',
  brightGreen: '#2a9d5c',
  brightYellow: '#c88a2a',
  brightBlue: '#4785ff',
  brightMagenta: '#9a5cf0',
  brightCyan: '#2596a0',
  brightWhite: '#212121'
};

const dracula: ITheme = {
  background: '#282a36',
  foreground: '#f8f8f2',
  cursor: '#f8f8f2',
  cursorAccent: '#282a36',
  selectionBackground: 'rgba(68, 71, 90, 0.75)',
  black: '#21222c',
  red: '#ff5555',
  green: '#50fa7b',
  yellow: '#f1fa8c',
  blue: '#bd93f9',
  magenta: '#ff79c6',
  cyan: '#8be9fd',
  white: '#f8f8f2',
  brightBlack: '#6272a4',
  brightRed: '#ff6e6e',
  brightGreen: '#69ff94',
  brightYellow: '#ffffa5',
  brightBlue: '#d6acff',
  brightMagenta: '#ff92df',
  brightCyan: '#a4ffff',
  brightWhite: '#ffffff'
};

const nord: ITheme = {
  background: '#2e3440',
  foreground: '#d8dee9',
  cursor: '#d8dee9',
  cursorAccent: '#2e3440',
  selectionBackground: 'rgba(76, 86, 106, 0.7)',
  black: '#3b4252',
  red: '#bf616a',
  green: '#a3be8c',
  yellow: '#ebcb8b',
  blue: '#81a1c1',
  magenta: '#b48ead',
  cyan: '#88c0d0',
  white: '#e5e9f0',
  brightBlack: '#4c566a',
  brightRed: '#d08770',
  brightGreen: '#b5d19c',
  brightYellow: '#f0d8a8',
  brightBlue: '#88c0d0',
  brightMagenta: '#c7a4c2',
  brightCyan: '#8fbcbb',
  brightWhite: '#eceff4'
};

const oneDark: ITheme = {
  background: '#282c34',
  foreground: '#abb2bf',
  cursor: '#528bff',
  cursorAccent: '#282c34',
  selectionBackground: 'rgba(62, 68, 81, 0.75)',
  black: '#282c34',
  red: '#e06c75',
  green: '#98c379',
  yellow: '#e5c07b',
  blue: '#61afef',
  magenta: '#c678dd',
  cyan: '#56b6c2',
  white: '#abb2bf',
  brightBlack: '#5c6370',
  brightRed: '#e88388',
  brightGreen: '#a8cc8c',
  brightYellow: '#dbab79',
  brightBlue: '#71bef2',
  brightMagenta: '#d290e4',
  brightCyan: '#66c2cd',
  brightWhite: '#ffffff'
};

const tokyoNight: ITheme = {
  background: '#1a1b26',
  foreground: '#c0caf5',
  cursor: '#c0caf5',
  cursorAccent: '#1a1b26',
  selectionBackground: 'rgba(51, 70, 124, 0.65)',
  black: '#15161e',
  red: '#f7768e',
  green: '#9ece6a',
  yellow: '#e0af68',
  blue: '#7aa2f7',
  magenta: '#bb9af7',
  cyan: '#7dcfff',
  white: '#a9b1d6',
  brightBlack: '#414868',
  brightRed: '#ff899d',
  brightGreen: '#b9f27c',
  brightYellow: '#ff9e64',
  brightBlue: '#8db0ff',
  brightMagenta: '#c7a9ff',
  brightCyan: '#a4daff',
  brightWhite: '#c0caf5'
};

const gruvboxDark: ITheme = {
  background: '#282828',
  foreground: '#ebdbb2',
  cursor: '#ebdbb2',
  cursorAccent: '#282828',
  selectionBackground: 'rgba(102, 92, 84, 0.65)',
  black: '#282828',
  red: '#cc241d',
  green: '#98971a',
  yellow: '#d79921',
  blue: '#458588',
  magenta: '#b16286',
  cyan: '#689d6a',
  white: '#a89984',
  brightBlack: '#928374',
  brightRed: '#fb4934',
  brightGreen: '#b8bb26',
  brightYellow: '#fabd2f',
  brightBlue: '#83a598',
  brightMagenta: '#d3869b',
  brightCyan: '#8ec07c',
  brightWhite: '#ebdbb2'
};

const solarizedDark: ITheme = {
  background: '#002b36',
  foreground: '#839496',
  cursor: '#93a1a1',
  cursorAccent: '#002b36',
  selectionBackground: 'rgba(7, 54, 66, 0.85)',
  black: '#073642',
  red: '#dc322f',
  green: '#859900',
  yellow: '#b58900',
  blue: '#268bd2',
  magenta: '#d33682',
  cyan: '#2aa198',
  white: '#eee8d5',
  brightBlack: '#586e75',
  brightRed: '#cb4b16',
  brightGreen: '#586e75',
  brightYellow: '#657b83',
  brightBlue: '#839496',
  brightMagenta: '#6c71c4',
  brightCyan: '#93a1a1',
  brightWhite: '#fdf6e3'
};

const solarizedLight: ITheme = {
  background: '#fdf6e3',
  foreground: '#657b83',
  cursor: '#586e75',
  cursorAccent: '#fdf6e3',
  selectionBackground: 'rgba(238, 232, 213, 0.9)',
  black: '#073642',
  red: '#dc322f',
  green: '#859900',
  yellow: '#b58900',
  blue: '#268bd2',
  magenta: '#d33682',
  cyan: '#2aa198',
  white: '#eee8d5',
  brightBlack: '#002b36',
  brightRed: '#cb4b16',
  brightGreen: '#586e75',
  brightYellow: '#657b83',
  brightBlue: '#839496',
  brightMagenta: '#6c71c4',
  brightCyan: '#93a1a1',
  brightWhite: '#fdf6e3'
};

const THEMES: Record<Theme, ITheme> = { dark, light };

const SCHEMES: Record<Exclude<TerminalColorScheme, 'auto'>, ITheme> = {
  dracula,
  nord,
  oneDark,
  tokyoNight,
  gruvboxDark,
  solarizedDark,
  solarizedLight
};

/** The xterm theme object for the app theme + selected terminal color scheme. */
export function xtermTheme(theme: Theme, scheme: TerminalColorScheme = 'auto'): ITheme {
  if (scheme === 'auto') return THEMES[theme];
  return SCHEMES[scheme] ?? THEMES[theme];
}

export function isLightTerminalScheme(theme: Theme, scheme: TerminalColorScheme = 'auto'): boolean {
  if (scheme === 'solarizedLight') return true;
  if (scheme === 'auto') return theme === 'light';
  return false;
}

export interface SearchColours {
  matchBackground: string;
  matchOverviewRuler: string;
  activeMatchBackground: string;
  activeMatchBorder: string;
  activeMatchColorOverviewRuler: string;
}

const SEARCH: Record<Theme, SearchColours> = {
  dark: {
    matchBackground: '#5a4a1e',
    matchOverviewRuler: '#e8c15a',
    activeMatchBackground: '#94701c',
    activeMatchBorder: '#f0d178',
    activeMatchColorOverviewRuler: '#f0d178'
  },
  light: {
    matchBackground: '#fbe7a8',
    matchOverviewRuler: '#b7791f',
    activeMatchBackground: '#f2b84b',
    activeMatchBorder: '#b7791f',
    activeMatchColorOverviewRuler: '#b7791f'
  }
};

export function searchColours(theme: Theme, scheme: TerminalColorScheme = 'auto'): SearchColours {
  return SEARCH[isLightTerminalScheme(theme, scheme) ? 'light' : 'dark'];
}
