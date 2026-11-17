import { formatNumber } from '@decay/ui';
import type { ProtectedBy } from '@/lib/db/types';
import type { KeptGroup } from '@/lib/shadow/report';
import { t } from '@/lib/strings';

/** Why an item is kept, in words ("playing audio"). */
export const protectedLabel = (by: ProtectedBy): string => t(`protected.${by}`);

/** A group on the review's "Why things were kept" card. */
export const keptGroupLabel = (group: KeptGroup): string => t(`kept.${group}`);

/** The groups, in the card's order. */
export const KEPT_GROUPS: KeptGroup[] = [
  'pinned',
  'audio',
  'active',
  'form',
  'rules',
  'hold',
  'other',
];

/** Whole active hours, never rounded up (an item is never shown older than it is). */
export const activeHours = (seconds: number) => Math.floor(seconds / 3600);

export function percent(rate: number | null): string {
  if (rate === null) return t('percent.none');
  const p = (r: number) => formatNumber(r, { style: 'percent', maximumFractionDigits: 0 });
  if (rate > 0 && rate < 0.01) return `<${p(0.01)}`;
  return p(Math.round(rate * 100) / 100);
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
