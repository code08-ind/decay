// What's New (P2.7, PRD F32, Designs/16): the notes of each version, shipped with the app
// so they show offline right after an update. Add an entry when releasing a version with
// something worth telling; a version without one shows nothing (the full changelog is the
// GitHub release text).
import { formatDate as formatLocaleDate } from '@decay/ui';

export type NoteIcon = 'windows' | 'summary' | 'transfer' | 'insights' | 'meeting' | 'search';

export interface ReleaseNotes {
  version: string;
  /** `YYYY-MM-DD`. */
  date: string;
  /** The one big thing (the green card), if there is one. */
  major?: { title: string; text: string };
  items: { icon: NoteIcon; title: string; text: string }[];
}

/** Newest first. */
export const RELEASES: ReleaseNotes[] = [
  {
    version: '0.1.0',
    date: '2026-11-17',
    major: {
      title: 'Windows of other apps, too',
      text: 'Decay can hide app windows you haven’t used in a while on macOS, and bring each one back exactly where it was.',
    },
    items: [
      {
        icon: 'summary',
        title: 'Your morning summary',
        text: 'What was archived, hidden and reopened yesterday, with a hint or two.',
      },
      {
        icon: 'meeting',
        title: 'Quiet during calls',
        text: 'Sweeps wait while you’re in a meeting or sharing your screen.',
      },
      {
        icon: 'transfer',
        title: 'Export and import',
        text: 'Take your archive with you as JSON, CSV or browser bookmarks.',
      },
      {
        icon: 'insights',
        title: 'Insights',
        text: 'See how many tabs you keep open, how often you need one back, and how fast you find it — all on this computer.',
      },
    ],
  },
];

/** The notes of `version`, if it has any. */
export function notesFor(version: string): ReleaseNotes | undefined {
  return RELEASES.find((r) => r.version === version);
}

/** "17 November 2026" in the app's language (the day as written, no time-zone shift). */
export function formatDate(day: string): string {
  const [y, m, d] = day.split('-').map(Number);
  if (!y || !m || !d) return day;
  return formatLocaleDate(Date.UTC(y, m - 1, d), {
    day: 'numeric',
    month: 'long',
    year: 'numeric',
    timeZone: 'UTC',
  });
}
