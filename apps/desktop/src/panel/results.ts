// The panel's result logic (P1.9), kept pure for tests: grouping, labels, highlighting,
// monograms and keyboard actions.

import { getLocale, logicalArrow } from '@decay/ui';
import type { PanelItem } from '../api';
import { t } from '../strings';

export interface ResultGroup {
  key: string;
  /** "Today · Google Chrome — Research" */
  label: string;
  items: PanelItem[];
  /** Restorable together (⌘↵ / "Restore all"). */
  batch: boolean;
}

const DAY = 86_400_000;

/** Midnight (local) of the day containing `t`. */
function startOfDay(t: number): number {
  const d = new Date(t);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** "Today", "Yesterday", "Last Tuesday" (within a week), else a date. */
export function dayLabel(at: number, now: number, locale = getLocale()): string {
  const days = Math.round((startOfDay(now) - startOfDay(at)) / DAY);
  if (days <= 0) return t('day.today');
  if (days === 1) return t('day.yesterday');
  const d = new Date(at);
  if (days < 7)
    return t('day.last', { weekday: d.toLocaleDateString(locale, { weekday: 'long' }) });
  const sameYear = d.getFullYear() === new Date(now).getFullYear();
  return d.toLocaleDateString(locale, {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
  });
}

/** The time column: "Today, 2:14 PM" or "Tue, Oct 28". */
export function timeLabel(at: number, now: number, locale = getLocale()): string {
  const d = new Date(at);
  if (startOfDay(at) === startOfDay(now)) {
    return t('day.todayAt', {
      time: d.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit' }),
    });
  }
  return d.toLocaleDateString(locale, { weekday: 'short', month: 'short', day: 'numeric' });
}

/**
 * Groups ranked results by local day and source window, keeping the ranking: groups appear
 * in the order of their best result, and items keep their order inside a group.
 */
export function groupResults(items: PanelItem[], now: number, locale?: string): ResultGroup[] {
  const groups = new Map<string, ResultGroup>();
  for (const item of items) {
    const at = item.archivedAt ?? now;
    const day = startOfDay(at);
    const key = `${day}|${item.sourceId ?? `app:${item.appName}`}`;
    let g = groups.get(key);
    if (!g) {
      const source = item.sourceTitle ? `${item.appName} — ${item.sourceTitle}` : item.appName;
      g = { key, label: `${dayLabel(at, now, locale)} · ${source}`, items: [], batch: false };
      groups.set(key, g);
    }
    g.items.push(item);
  }
  for (const g of groups.values()) g.batch = g.items.length > 1 && g.items.some((i) => i.batch);
  return [...groups.values()];
}

/** The words to highlight: the query without operators (`site:x`, `before:…`). */
export function searchWords(query: string): string[] {
  return query
    .split(/\s+/)
    .filter((w) => w && !/^(app|site|before|after|in):/i.test(w))
    .map((w) => w.replace(/["*]/g, '').toLowerCase())
    .filter((w) => w.length > 0);
}

/** A character without accents, lowercased (FTS5's `unicode61` folds the same way). */
function foldChar(c: string): string {
  return c.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
}

const WORD = /[\p{L}\p{N}]/u;

/**
 * `text` split into plain and matching parts: word-prefix matches of each search word,
 * ignoring case and accents ("cafe" marks "Café"), like the search itself.
 */
export function highlight(text: string, words: string[]): { text: string; match: boolean }[] {
  const wanted = words.map((w) => [...w].map(foldChar).join('')).filter(Boolean);
  if (wanted.length === 0) return [{ text, match: false }];
  const chars = [...text];
  const folded = chars.map(foldChar);
  const marked = new Array<boolean>(chars.length).fill(false);
  for (let i = 0; i < chars.length; i++) {
    // Matches start at a word boundary ("pric" marks "pricing", not "Repricing").
    if (i > 0 && WORD.test(chars[i - 1] ?? '')) continue;
    for (const w of wanted) {
      // Does the folded text starting at i begin with w?
      let j = i;
      let got = '';
      while (j < chars.length && got.length < w.length) got += folded[j++];
      if (!got.startsWith(w)) continue;
      // Mark the rest of the word too.
      while (j < chars.length && WORD.test(chars[j] ?? '')) j++;
      for (let k = i; k < j; k++) marked[k] = true;
    }
  }
  const parts: { text: string; match: boolean }[] = [];
  chars.forEach((c, i) => {
    const last = parts[parts.length - 1];
    if (last && last.match === marked[i]) last.text += c;
    else parts.push({ text: c, match: marked[i] ?? false });
  });
  return parts.length ? parts : [{ text, match: false }];
}

/** A favicon stand-in (no network): a letter and one of four tints, stable per site. */
export function monogram(item: Pick<PanelItem, 'domain' | 'title'>): {
  letter: string;
  tint: 1 | 2 | 3 | 4;
} {
  const base = item.domain ?? item.title;
  const letter = (base.match(/[\p{L}\p{N}]/u)?.[0] ?? '?').toUpperCase();
  let h = 0;
  for (const c of base) h = (h * 31 + (c.codePointAt(0) ?? 0)) >>> 0;
  return { letter, tint: ((h % 4) + 1) as 1 | 2 | 3 | 4 };
}

/** RFC 3492 Punycode → Unicode for one `xn--` label (undefined if it isn't valid). */
function punycodeLabel(label: string): string | undefined {
  const input = label.slice(4);
  const base = 36;
  const out: number[] = [];
  const delim = input.lastIndexOf('-');
  for (let i = 0; i < Math.max(delim, 0); i++) out.push(input.charCodeAt(i));
  let n = 128;
  let bias = 72;
  let i = 0;
  const adapt = (delta: number, points: number, first: boolean) => {
    let d = first ? Math.floor(delta / 700) : delta >> 1;
    d += Math.floor(d / points);
    let k = 0;
    while (d > 455) {
      d = Math.floor(d / 35);
      k += base;
    }
    return k + Math.floor((36 * d) / (d + 38));
  };
  for (let pos = delim > 0 ? delim + 1 : 0; pos < input.length; ) {
    const oldi = i;
    let w = 1;
    for (let k = base; ; k += base) {
      if (pos >= input.length) return undefined;
      const c = input.charCodeAt(pos++);
      const digit = c - 48 < 10 ? c - 22 : c - 65 < 26 ? c - 65 : c - 97 < 26 ? c - 97 : base;
      if (digit >= base) return undefined;
      i += digit * w;
      const t = k <= bias ? 1 : k >= bias + 26 ? 26 : k - bias;
      if (digit < t) break;
      w *= base - t;
    }
    bias = adapt(i - oldi, out.length + 1, oldi === 0);
    n += Math.floor(i / (out.length + 1));
    if (n > 0x10ffff || !Number.isSafeInteger(i)) return undefined; // not a real label
    i %= out.length + 1;
    out.splice(i++, 0, n);
  }
  return String.fromCodePoint(...out);
}

/** A host as people read it (`xn--strae-oqa.example` → `straße.example`). */
export function displayHost(host: string): string {
  return host
    .split('.')
    .map((l) => (l.toLowerCase().startsWith('xn--') ? (punycodeLabel(l) ?? l) : l))
    .join('.');
}

/** The URL shown under the title: host / path, decoded (no scheme, no query). */
export function displayUrl(url: string | null): string {
  if (!url) return '';
  try {
    const u = new URL(url);
    let path = u.pathname.replace(/\/$/, '');
    try {
      path = decodeURIComponent(path);
    } catch {
      // Not valid percent-encoding: show it as it is.
    }
    const host = displayHost(u.host);
    return path ? `${host} / ${path.slice(1)}` : host;
  } catch {
    return url;
  }
}

export type PanelAction =
  | 'next'
  | 'previous'
  | 'first'
  | 'last'
  | 'restore'
  | 'restoreGroup'
  | 'restoreNewWindow'
  | 'protect'
  | 'details';

/** The panel's keys (UI Design Guidelines §9); `mod` is ⌘ on macOS, Ctrl elsewhere. */
export function panelAction(
  e: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'> & {
    isComposing?: boolean;
  },
  mac: boolean,
): PanelAction | null {
  // Keys confirming an input method's candidate (Japanese, Chinese…) belong to it.
  if (e.isComposing || e.key === 'Process') return null;
  const mod = mac ? e.metaKey : e.ctrlKey;
  // ⌘→ "into the details" is ⌘← right to left.
  switch (logicalArrow(e.key)) {
    case 'ArrowDown':
      return 'next';
    case 'ArrowUp':
      return 'previous';
    case 'PageUp':
      return 'first';
    case 'PageDown':
      return 'last';
    case 'Enter':
      if (mod) return 'restoreGroup';
      if (e.altKey) return 'restoreNewWindow';
      return 'restore';
    case 'p':
    case 'P':
      return mod ? 'protect' : null;
    case 'i':
    case 'I':
      return mod ? 'details' : null;
    case 'ArrowRight':
      return mod ? 'details' : null;
    default:
      return null;
  }
}
