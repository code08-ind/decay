// Onboarding (F21) without the UI: the steps, the presets and what each browser row shows.
import type { DetectedBrowser, Preset } from '../api';
import { type Key, t } from '../strings';

export const STEPS = [
  { id: 'welcome' },
  { id: 'pace' },
  { id: 'connect' },
  { id: 'windows' },
  { id: 'shadow' },
  { id: 'done' },
] as const;

export type StepId = (typeof STEPS)[number]['id'];

/** A step's name in the sidebar, and its kicker above the title. */
export const stepTitle = (id: StepId): string => t(`onboarding.step.${id}`);
export const stepKicker = (id: StepId): string => t(`onboarding.kicker.${id}`);

/** Active hours to Stale / Hidden / Archived (PRD §1.1; decay-core `Thresholds`). */
export const PRESETS: {
  id: Preset;
  name: Key;
  blurb: Key;
  hours: { stale: number; hidden: number; archive: number };
}[] = [
  {
    id: 'gentle',
    name: 'onboarding.preset.gentle',
    blurb: 'onboarding.preset.gentle.blurb',
    hours: { stale: 24, hidden: 60, archive: 96 },
  },
  {
    id: 'balanced',
    name: 'onboarding.preset.balanced',
    blurb: 'onboarding.preset.balanced.blurb',
    hours: { stale: 12, hidden: 36, archive: 60 },
  },
  {
    id: 'aggressive',
    name: 'onboarding.preset.aggressive',
    blurb: 'onboarding.preset.aggressive.blurb',
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
      return b.profiles > 1
        ? t('onboarding.row.profiles', { n: b.profiles })
        : t('onboarding.row.connected');
    case 'install':
      return b.hostInstalled ? t('onboarding.row.waiting') : t('onboarding.row.detected');
    default:
      return t('onboarding.row.notInstalled');
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
    return t('onboarding.host.missing');
  }
  return t('onboarding.host.failed');
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
