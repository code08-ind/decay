import type { DecayDB } from '../db/db';
import type { MetricsDay } from '../db/types';
import { emptyMetrics, localDay } from '../shadow/shadow';

/**
 * On-device success metrics (P0.8, PRD §9 / F34). Everything here stays in the browser;
 * the CSV export carries aggregates only, and only when the user saves it.
 */

/** Samples kept per day for medians (hourly open-tab counts, search → restore times). */
export const MAX_SAMPLES = 48;

/** Longer search → restore times are an idle popup, not a search; they're not recorded. */
export const MAX_SEARCH_TO_RESTORE_MS = 120_000;

export { median } from '@decay/ui/metrics';

import { median } from '@decay/ui/metrics';

const pushSample = (list: number[] | undefined, value: number) =>
  [...(list ?? []), value].slice(-MAX_SAMPLES);

/**
 * Hourly open tabs / windows sample (DB Design §3.10 "median open tabs sampled hourly"):
 * at most one sample per local hour, taken while the browser is in use. Mutates `row`.
 * Returns whether a sample was added.
 */
export function sampleOpenCounts(
  row: MetricsDay,
  now: number,
  counts: { tabs: number; windows: number },
): boolean {
  // No normal windows (browser starting, or only app/popup windows): not a sample.
  if (counts.tabs <= 0) return false;
  const hour = new Date(now).getHours();
  if (row.sampledHours?.includes(hour)) return false;
  row.sampledHours = [...(row.sampledHours ?? []), hour];
  row.openTabsSamples = pushSample(row.openTabsSamples, counts.tabs);
  row.openWindowsSamples = pushSample(row.openWindowsSamples, counts.windows);
  row.openTabsP50 = median(row.openTabsSamples);
  row.openWindowsP50 = median(row.openWindowsSamples);
  return true;
}

/** A search in the popup (once per popup opening). */
export async function recordSearch(db: DecayDB, now = Date.now()): Promise<void> {
  await db.transaction('rw', db.metrics, async () => {
    const day = localDay(now);
    const row = (await db.metrics.get(day)) ?? emptyMetrics(day);
    row.searchCount = (row.searchCount ?? 0) + 1;
    await db.metrics.put(row);
  });
}

/** Time from the first keystroke of a search to restoring one of its results (PRD §9 < 5 s). */
export async function recordSearchToRestore(
  db: DecayDB,
  ms: number,
  now = Date.now(),
): Promise<void> {
  if (!Number.isFinite(ms) || ms < 0 || ms > MAX_SEARCH_TO_RESTORE_MS) return;
  await db.transaction('rw', db.metrics, async () => {
    const day = localDay(now);
    const row = (await db.metrics.get(day)) ?? emptyMetrics(day);
    row.searchToRestoreSamples = pushSample(row.searchToRestoreSamples, Math.round(ms));
    row.searchToRestoreMsP50 = median(row.searchToRestoreSamples);
    await db.metrics.put(row);
  });
}
