import type { Thresholds } from '../db/types';

/** Allowed range for each threshold, in active hours (Docs/API Design.md `update_settings`). */
export const THRESHOLD_MIN_H = 1;
export const THRESHOLD_MAX_H = 720;

export function thresholdsProblem(t: Thresholds | undefined | null): string | null {
  const values = [t?.stale, t?.hidden, t?.archive];
  const ok = values.every(
    (h) =>
      typeof h === 'number' && Number.isFinite(h) && h >= THRESHOLD_MIN_H && h <= THRESHOLD_MAX_H,
  );
  if (!ok || !t || !(t.stale < t.hidden && t.hidden < t.archive)) {
    return `Thresholds must be ${THRESHOLD_MIN_H}–${THRESHOLD_MAX_H} h and ordered stale < hidden < archive`;
  }
  return null;
}

export const areValidThresholds = (t: Thresholds | undefined | null): t is Thresholds =>
  thresholdsProblem(t) === null;
