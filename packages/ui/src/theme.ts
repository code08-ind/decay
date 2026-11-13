/** Theme preference (F19): follow the OS, or force one. Stored as setting `theme`. */
export type ThemePreference = 'system' | 'light' | 'dark';

export function isThemePreference(v: unknown): v is ThemePreference {
  return v === 'system' || v === 'light' || v === 'dark';
}

/**
 * Applies a preference: `light` / `dark` set `<html data-theme>` (tokens.css switches on
 * it); `system` removes it so `prefers-color-scheme` decides. Unknown values mean system.
 */
export function applyTheme(
  pref: unknown,
  root: HTMLElement = document.documentElement,
): ThemePreference {
  const theme = isThemePreference(pref) ? pref : 'system';
  if (theme === 'system') delete root.dataset.theme;
  else root.dataset.theme = theme;
  return theme;
}

/** The theme in effect now (resolving `system` through the OS setting). */
export function effectiveTheme(
  pref: ThemePreference,
  prefersDark: boolean = globalThis.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false,
): 'light' | 'dark' {
  return pref === 'system' ? (prefersDark ? 'dark' : 'light') : pref;
}
