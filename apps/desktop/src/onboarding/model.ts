// Onboarding (F21) without the UI: the steps, the presets and what each browser row shows.
import type { DetectedBrowser, Preset } from '../api';

export const STEPS = [
  { id: 'welcome', title: 'Welcome', kicker: 'Introduction' },
  { id: 'pace', title: 'Choose pace', kicker: 'Pace' },
  { id: 'connect', title: 'Connect browsers', kicker: 'Browsers' },
  { id: 'windows', title: 'Window access', kicker: 'Windows' },
  { id: 'shadow', title: 'Shadow mode', kicker: 'Shadow mode' },
  { id: 'done', title: 'All set', kicker: 'Done' },
] as const;

export type StepId = (typeof STEPS)[number]['id'];

/** Active hours to Stale / Hidden / Archived (PRD §1.1; decay-core `Thresholds`). */
export const PRESETS: {
  id: Preset;
  name: string;
  blurb: string;
  hours: { stale: number; hidden: number; archive: number };
}[] = [
  {
    id: 'gentle',
    name: 'Gentle',
    blurb: 'For people who keep research open for days.',
    hours: { stale: 24, hidden: 60, archive: 96 },
  },
  {
    id: 'balanced',
    name: 'Balanced',
    blurb: 'A good default for most work.',
    hours: { stale: 12, hidden: 36, archive: 60 },
  },
  {
    id: 'aggressive',
    name: 'Aggressive',
    blurb: "Keep only what you're using today.",
    hours: { stale: 8, hidden: 24, archive: 40 },
  },
];

export type RowState = 'connected' | 'install' | 'missing';

/** What a browser row offers: connected, an install button, or "Not found". */
export function rowState(b: DetectedBrowser): RowState {
  if (b.extensionConnected) return 'connected';
  return b.installed ? 'install' : 'missing';
}

/** The line under a browser's name. */
export function rowDetail(b: DetectedBrowser): string {
  switch (rowState(b)) {
    case 'connected':
      return b.profiles > 1 ? `${b.profiles} profiles connected` : 'Extension connected';
    case 'install':
      return b.hostInstalled ? 'Detected · waiting for the extension' : 'Detected';
    default:
      return 'Not installed';
  }
}

/** Rows in the order the design lists them: connected, then installable, then missing. */
export function sortRows(rows: DetectedBrowser[]): DetectedBrowser[] {
  const rank: Record<RowState, number> = { connected: 0, install: 1, missing: 2 };
  return rows
    .map((b, i) => ({ b, i }))
    .sort((x, y) => rank[rowState(x.b)] - rank[rowState(y.b)] || x.i - y.i)
    .map(({ b }) => b);
}

/** What to tell the user when the browser connector (native host) couldn't be set up. */
export function hostProblem(e: unknown): string {
  const code = e && typeof e === 'object' && 'code' in e ? String(e.code) : '';
  if (code === 'NOT_FOUND') {
    return "This copy of Decay doesn't include its browser connector, so browsers can't connect to it yet.";
  }
  return "Decay couldn't set up its browser connector, so browsers can't connect to it yet. Restarting Decay usually fixes this.";
}

/** The step after `step` (clamped). */
export function next(step: number): number {
  return Math.min(step + 1, STEPS.length - 1);
}

/** The step before `step`; there's no going back once onboarding is complete. */
export function back(step: number, completed: boolean): number {
  if (completed) return step;
  return Math.max(step - 1, 0);
}
