import { browser } from 'wxt/browser';

/** Locally cached favicon via the `favicon` permission — no network request. Firefox has
 * no such cache: no icon (the UI shows its letter instead). */
export function faviconFor(pageUrl: string, size = 32): string | undefined {
  if (import.meta.env.FIREFOX || !/^https?:/i.test(pageUrl)) return undefined;
  const base = browser.runtime.getURL('/_favicon/' as never);
  return `${base}?pageUrl=${encodeURIComponent(pageUrl)}&size=${size}`;
}
