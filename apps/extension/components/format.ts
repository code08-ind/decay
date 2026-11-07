import type { ProtectedBy } from '@/lib/db/types';
import type { KeptGroup } from '@/lib/shadow/report';

export const PROTECTED_LABEL: Record<ProtectedBy, string> = {
  pinned: 'pinned',
  activeTab: 'open in a window',
  audible: 'playing audio',
  formInput: 'unsaved work',
  download: 'downloading',
  call: 'in a call',
  unsaved: 'unsaved changes',
  unknownSaveState: 'save state unknown',
  rule: 'your rule',
  hold: 'on hold',
  meeting: 'in a meeting',
  grace: 'recently restored',
  unrestorable: "can't be reopened",
};

export const KEPT_GROUP_LABEL: Record<KeptGroup, string> = {
  pinned: 'Pinned tabs',
  audio: 'Playing audio',
  active: 'Open in a window',
  form: 'Unsaved work',
  rules: 'Rules & safelist',
  hold: 'On hold',
  other: "Can't be reopened",
};

/** Whole active hours, never rounded up (an item is never shown older than it is). */
export const activeHours = (seconds: number) => Math.floor(seconds / 3600);

export function percent(rate: number | null): string {
  if (rate === null) return '—';
  if (rate > 0 && rate < 0.01) return '<1%';
  return `${Math.round(rate * 100)}%`;
}

/** `example.com/path` for display; falls back to the raw URL. */
export function shortUrl(url: string | undefined): string {
  if (!url) return '';
  try {
    const u = new URL(url);
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return url;
    const path = u.pathname === '/' ? '' : u.pathname;
    return `${u.hostname.replace(/^www\./, '')}${path}`;
  } catch {
    return url;
  }
}

export function minutesUntil(at: number | null, now: number): number | null {
  if (at === null) return null;
  return Math.max(0, Math.ceil((at - now) / 60_000));
}

const AVATARS = [
  'bg-avatar-1 text-avatar-1-ink',
  'bg-avatar-2 text-avatar-2-ink',
  'bg-avatar-3 text-avatar-3-ink',
  'bg-avatar-4 text-avatar-4-ink',
];

/** Letter-tile colours for items without a favicon, stable per site. */
export function avatarClass(key: string): string {
  let hash = 0;
  for (const ch of key) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return AVATARS[hash % AVATARS.length] ?? AVATARS[0] ?? '';
}
