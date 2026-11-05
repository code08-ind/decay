import { browser } from 'wxt/browser';

/** Locally cached favicon via the `favicon` permission — no network request. */
export function faviconFor(pageUrl: string, size = 32): string | undefined {
  if (!/^https?:/i.test(pageUrl)) return undefined;
  const base = browser.runtime.getURL('/_favicon/' as never);
  return `${base}?pageUrl=${encodeURIComponent(pageUrl)}&size=${size}`;
}
