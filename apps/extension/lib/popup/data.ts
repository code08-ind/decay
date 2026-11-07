import type { DecayDB } from '../db/db';
import type { Item, Mode, Sweep } from '../db/types';
import { hostOf, ruleMatches } from '../policy/match';
import type { ProbeRecord } from '../probe/probe';
import type { Settings } from '../settings/settings';
import { loadShadowReport } from '../shadow/report';
import { emptyMetrics, localDay } from '../shadow/shadow';
import { recentSweeps } from '../sweep/executor';
import type { TabInfo } from '../tracker/tabs';

/** Designs/15 shows three; Chrome caps popups at 600 px tall. */
export const RECENT_COUNT = 3;

export interface PopupData {
  mode: Mode;
  holdUntil: number | null;
  shadow: { dayNumber: number; targetDays: number; ready: boolean; wouldArchive: number };
  openTabs: number;
  archivedToday: number;
  lastSweep: { sweep: Sweep; remaining: number } | null;
  recent: Item[];
  /** Every archived item, for the search index. */
  archived: Item[];
  /** item id → window at archive time (search grouping / batches). */
  windowOf: Map<string, string | undefined>;
  /** The tab the popup was opened on: its site and whether a protect rule covers it. */
  currentSite: { host: string; url: string; protected: boolean } | null;
  /** Decay can check pages for typed input (site access granted, P0.7). */
  probeAccess: boolean;
}

export async function loadPopupData(
  db: DecayDB,
  all: Settings,
  openTabs: readonly TabInfo[],
  currentTab: TabInfo | undefined,
  now: number,
  probe: { probes?: Readonly<ProbeRecord>; probeAccess?: boolean } = {},
): Promise<PopupData> {
  const [report, metrics, [last], archived, rules] = await Promise.all([
    loadShadowReport(db, all, openTabs, now, probe),
    db.metrics.get(localDay(now)),
    recentSweeps(db, 1),
    db.items.where('state').equals('archived').toArray(),
    db.rules.toArray(),
  ]);
  archived.sort((a, b) => (b.archivedAt ?? 0) - (a.archivedAt ?? 0));

  const sweepIds = [
    ...new Set(archived.map((i) => i.lastSweepId).filter((id) => id !== undefined)),
  ];
  const sweeps = await db.sweeps.bulkGet(sweepIds as string[]);
  const windowOf = new Map<string, string | undefined>();
  for (const sweep of sweeps) {
    for (const e of sweep?.items ?? []) windowOf.set(e.itemId, e.windowExternalId);
  }

  let lastSweep: PopupData['lastSweep'] = null;
  if (last) {
    const remaining = archived.filter((i) => i.lastSweepId === last.id).length;
    lastSweep = { sweep: last, remaining };
  }

  let currentSite: PopupData['currentSite'] = null;
  const url = currentTab?.url;
  const host = hostOf(url);
  if (url && host && /^https?:/i.test(url)) {
    const target = { appId: '', url, title: currentTab?.title ?? '' };
    const isProtected = rules.some((r) => r.action === 'protect' && ruleMatches(r, target));
    currentSite = { host: host.replace(/^www\./, ''), url, protected: isProtected };
  }

  return {
    mode: all.mode,
    holdUntil: all.holdUntil,
    shadow: {
      dayNumber: report.dayNumber,
      targetDays: report.targetDays,
      ready: report.ready,
      wouldArchive: report.wouldArchive.length,
    },
    openTabs: openTabs.length,
    archivedToday: (metrics ?? emptyMetrics('')).archivedCount ?? 0,
    lastSweep,
    recent: archived.slice(0, RECENT_COUNT),
    archived,
    windowOf,
    currentSite,
    probeAccess: report.probeAccess,
  };
}
