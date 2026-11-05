import type { Settings } from '../settings/settings';

/** Applies the theme setting: `system` follows the OS (tokens.css media query). */
export function applyTheme(theme: Settings['theme'], root = document.documentElement): void {
  if (theme === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', theme);
}
