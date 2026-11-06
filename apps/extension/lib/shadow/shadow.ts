import { uuidv7 } from 'uuidv7';
import type { Item, ItemEvent, MetricsDay, Sweep } from '../db/types';
import type { Decision } from '../policy/policy';

/** Shadow mode lasts 7 calendar days with ≥ 1 h of active time each (PRD F2). */
export const SHADOW_DAYS = 7;
export const ACTIVE_DAY_SECONDS = 3600;
/** Whether the review page offers automatic mode (real sweeps exist since P0.5). */
export const AUTOMATIC_MODE_AVAILABLE = true;

/** Simulated sweeps run at most every 15 min (Software Design §4.3). */
export const SHADOW_SWEEP_INTERVAL_MS = 15 * 60_000;

/** Local calendar day, `YYYY-MM-DD`. */
export function localDay(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function emptyMetrics(day: string): MetricsDay {
  return {
    day,
    activeSeconds: 0,
    openTabsP50: 0,
    openWindowsP50: 0,
    archivedCount: 0,
    restored24hCount: 0,
    undoCount: 0,
    searchCount: 0,
    searchToRestoreMsP50: 0,
    autoModeOn: 0,
    shadowFlaggedCount: 0,
    shadowRegretCount: 0,
  };
}

/** Calendar days since `sinceTs` (inclusive) that had at least 1 h of active time. */
export function countActiveDays(metrics: readonly MetricsDay[], sinceTs: number | null): number {
  const since = sinceTs === null ? '' : localDay(sinceTs);
  return metrics.filter((m) => m.day >= since && (m.activeSeconds ?? 0) >= ACTIVE_DAY_SECONDS)
    .length;
}

export interface ShadowSweepPlan {
  sweep: Sweep;
  flagged: Item[];
  events: ItemEvent[];
}

/**
 * A simulated sweep: picks the oldest open items whose decision is `archive` and that
 * aren't flagged yet, records a shadow sweep and a `shadow_would_archive` event each.
 * Nothing is closed. Returns null when there's nothing to flag.
 */
export function planShadowSweep(
  items: readonly Item[],
  decisions: ReadonlyMap<string, Decision>,
  now: number,
  maxItems: number,
  trigger: Sweep['trigger'] = 'scheduled',
): ShadowSweepPlan | null {
  const candidates = items
    .filter((i) => i.shadowFlaggedAt === undefined && decisions.get(i.id)?.action === 'archive')
    .sort((a, b) => b.activeAgeS - a.activeAgeS)
    .slice(0, Math.max(0, maxItems));
  if (!candidates.length) return null;
  const sweep: Sweep = {
    id: uuidv7(),
    startedAt: now,
    finishedAt: now,
    trigger,
    isShadow: 1,
    status: 'done',
    itemCount: candidates.length,
    failedCount: 0,
    items: candidates.map((i) => ({
      itemId: i.id,
      action: 'archive',
      prevState: i.state,
      result: 'skipped',
    })),
  };
  const events: ItemEvent[] = [];
  for (const item of candidates) {
    const decision = decisions.get(item.id);
    item.shadowFlaggedAt = now;
    events.push({
      itemId: item.id,
      type: 'shadow_would_archive',
      sweepId: sweep.id,
      at: now,
      reason: decision?.action === 'archive' ? decision.reason : undefined,
    });
  }
  return { sweep, flagged: candidates, events };
}
