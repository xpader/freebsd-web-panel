// Terminal color themes — switched by panel theme (dark/light).

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

const LIGHT = {
  background: '#fafbfc',
  foreground: '#24292f',
  cursor: '#24292f',
  cursorAccent: '#fafbfc',
  selectionBackground: '#4d99f055',
  black: '#24292f',
  red: '#cf222e',
  green: '#1a7f37',
  yellow: '#9a6700',
  blue: '#0969da',
  magenta: '#8250df',
  cyan: '#1b7c83',
  white: '#6e7781',
  brightBlack: '#5c6573',
  brightRed: '#a40e26',
  brightGreen: '#2da44e',
  brightYellow: '#bf8700',
  brightBlue: '#218bff',
  brightMagenta: '#a475f4',
  brightCyan: '#3192aa',
  brightWhite: '#1c2024',
};

export function termTheme(effective) {
  return effective === 'light' ? LIGHT : DARK;
}
