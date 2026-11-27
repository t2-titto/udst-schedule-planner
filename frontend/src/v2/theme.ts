export type ThemeMode = 'light' | 'dark'

const THEME_KEY = 'planie:theme'

export function readTheme(): ThemeMode {
  try {
    const v = window.localStorage.getItem(THEME_KEY)
    if (v === 'dark' || v === 'light') return v
  } catch { /* ignore */ }
  return 'light'
}

export function applyTheme(mode: ThemeMode): void {
  try {
    document.documentElement.setAttribute('data-theme', mode)
    window.localStorage.setItem(THEME_KEY, mode)
  } catch { /* ignore */ }
}
