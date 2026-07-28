// Terminal color themes — switched by panel theme (dark/light).
//
// Dark: custom dark palette matching the panel's dark theme.
// Light: Catppuccin Latte — a calm, modern pastel theme on cool gray-blue.
//        Designed for good TUI readability with distinct ANSI 16 colors.

const DARK = {
  background: '#0b0e14',
  foreground: '#d6dbe5',
  cursor: '#d6dbe5',
  cursorAccent: '#0b0e14',
  selectionBackground: '#264f78aa',
  black: '#010601',
  red: '#ea3431',
  green: '#3b914a',
  yellow: '#b5e028',
  blue: '#3a7dd2',
  magenta: '#c335db',
  cyan: '#22c8c8',
  white: '#d6dbe5',
  brightBlack: '#5c6370',
  brightRed: '#f76e6c',
  brightGreen: '#5fd962',
  brightYellow: '#e5f26d',
  brightBlue: '#5d9bf4',
  brightMagenta: '#d670f0',
  brightCyan: '#4afafa',
  brightWhite: '#ffffff',
};

// Catppuccin Latte — cool gray-blue background, high-contrast ANSI accents.
// All 16 colors chosen for clear distinction on light surfaces.
const LIGHT = {
  background: '#eff1f5',
  foreground: '#4c4f69',
  cursor: '#4c4f69',
  cursorAccent: '#eff1f5',
  selectionBackground: '#dce0e8',
  black: '#5c5f77',
  red: '#d20f39',
  green: '#40a02b',
  yellow: '#df8e1d',
  blue: '#1e66f5',
  magenta: '#ea76cb',
  cyan: '#179299',
  white: '#acb0be',
  brightBlack: '#6c6f85',
  brightRed: '#d20f39',
  brightGreen: '#40a02b',
  brightYellow: '#df8e1d',
  brightBlue: '#1e66f5',
  brightMagenta: '#ea76cb',
  brightCyan: '#179299',
  brightWhite: '#bcc0cc',
};

export function termTheme(effective) {
  return effective === 'light' ? LIGHT : DARK;
}
