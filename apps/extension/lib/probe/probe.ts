import type { TabSignals } from '../policy/policy';
import type { FrameForms, FrameUnload } from './inspect';

/**
 * Form / `beforeunload` probe (P0.7, PRD §7.3, Software Design §4.7 `probe.ts`).
 *
 * - `dirty`: typed input, or a "Leave site?" prompt would show → protected (`formInput`).
 * - `clean`: checked, nothing to lose → normal archive threshold.
 * - `unknown`: couldn't check (no site access, injection blocked, rich-text editor in a page
 *   loaded before Decay could watch it) → 2× archive threshold.
 */
export type ProbeVerdict = 'clean' | 'dirty' | 'unknown';

/** Site access for the probe: all web pages, only the sites the user picked, or none. */
export type ProbeAccess = 'all' | 'some' | 'none';

export interface ProbeEntry {
  /** URL the verdict is for: a navigation invalidates it. */
  url: string;
  at: number;
  verdict: ProbeVerdict;
}

/** Cached verdicts per tab id (persisted in `storage.session`, so pages can read them). */
export type ProbeRecord = Record<string, ProbeEntry>;

/** Clean verdicts are re-checked after this long. */
export const PROBE_TTL_MS = 30 * 60_000;
/**
 * "Couldn't check" (site access may have been granted) and "dirty" (an upload may have
 * finished) are re-checked sooner.
 */
export const PROBE_UNKNOWN_TTL_MS = 10 * 60_000;
/** Per-tab probe timeout: a hung page counts as unknown. */
export const PROBE_TIMEOUT_MS = 3_000;
/** A tick stops starting new probes after this long; the rest wait for the next tick. */
export const PROBE_TICK_BUDGET_MS = 5_000;
/** At most this many tabs are probed per tick (oldest first); the rest wait for the next. */
export const MAX_PROBES_PER_TICK = 40;
export const PROBE_CONCURRENCY = 6;

/** Only web pages can hold typed input we'd need to check. */
export const isWebUrl = (url: string | undefined): boolean => /^https?:/i.test(url ?? '');

/** One frame's answer from `scripting.executeScript` (untrusted until validated). */
export interface FrameResult {
  frameId: number;
  result?: unknown;
}

const isFrameForms = (r: unknown): r is FrameForms =>
  typeof r === 'object' &&
  r !== null &&
  typeof (r as FrameForms).tracked === 'boolean' &&
  typeof (r as FrameForms).dirty === 'boolean' &&
  typeof (r as FrameForms).unknown === 'boolean' &&
  ((r as FrameForms).frameOrigins === undefined ||
    (Array.isArray((r as FrameForms).frameOrigins) &&
      ((r as FrameForms).frameOrigins as unknown[]).every((o) => typeof o === 'string')));

const isFrameUnload = (r: unknown): r is FrameUnload =>
  typeof r === 'object' &&
  r !== null &&
  typeof (r as FrameUnload).tracked === 'boolean' &&
  typeof (r as FrameUnload).handlers === 'number' &&
  typeof (r as FrameUnload).activated === 'boolean' &&
  ((r as FrameUnload).uploads === undefined || typeof (r as FrameUnload).uploads === 'number');

/**
 * Combines per-frame results into one verdict. The top frame (id 0) must have answered both
 * checks; any frame answering garbage makes the verdict unknown. Dirty anywhere wins.
 * Frames the browser couldn't inject into don't appear at all (best effort for subframes).
 */
export function combineProbe(
  forms: readonly FrameResult[],
  unload: readonly FrameResult[],
  /** With partial site access: whether Decay may check pages from this origin. */
  canCheckOrigin?: (origin: string) => boolean,
): ProbeVerdict {
  const topForms = forms.some((f) => f.frameId === 0);
  const topUnload = unload.some((f) => f.frameId === 0);
  const formsOk = forms.every((f) => isFrameForms(f.result));
  const unloadOk = unload.every((f) => isFrameUnload(f.result));
  const formResults = forms.map((f) => f.result).filter(isFrameForms);
  const unloadResults = unload.map((f) => f.result).filter(isFrameUnload);
  // Dirty is trusted even from a partial answer: protecting is always safe.
  if (formResults.some((f) => f.dirty)) return 'dirty';
  if (unloadResults.some((u) => u.handlers > 0 && u.activated)) return 'dirty';
  if (unloadResults.some((u) => (u.uploads ?? 0) > 0)) return 'dirty'; // closing cancels it
  if (!topForms || !topUnload || !formsOk || !unloadOk) return 'unknown';
  if (formResults.some((f) => f.unknown)) return 'unknown';
  // Embedded pages from sites Decay may not check are skipped by the browser without an
  // error: their typing or uploads would be invisible.
  if (canCheckOrigin && formResults.some((f) => f.frameOrigins?.some((o) => !canCheckOrigin(o)))) {
    return 'unknown';
  }
  // Without the watcher (page loaded before it was registered) `addEventListener`
  // beforeunload handlers are invisible; after user activation the page might prompt.
  if (unloadResults.some((u) => !u.tracked && u.activated)) return 'unknown';
  return 'clean';
}

/** The policy signal for a verdict (missing = never checked = unknown). */
export function formSignal(verdict: ProbeVerdict | undefined): TabSignals['formInput'] {
  if (verdict === 'dirty') return true;
  if (verdict === 'clean') return false;
  return 'unknown';
}

/**
 * The cached verdict for a tab, if still valid: same URL, not expired, and the tab hasn't
 * been used since (typing needs the tab in front, which also resets its age).
 */
export function cachedVerdict(
  record: Readonly<ProbeRecord>,
  tab: { id?: number; url?: string; pendingUrl?: string } | undefined,
  lastUsedAt: number,
  now: number,
): ProbeVerdict | undefined {
  if (tab?.id === undefined) return undefined;
  const entry = record[String(tab.id)];
  if (!entry) return undefined;
  const url = tab.url || tab.pendingUrl;
  if (entry.url !== url) return undefined;
  if (lastUsedAt >= entry.at) return undefined;
  const ttl = entry.verdict === 'clean' ? PROBE_TTL_MS : PROBE_UNKNOWN_TTL_MS;
  if (now - entry.at >= ttl || entry.at > now) return undefined;
  return entry.verdict;
}

/** Keeps only well-formed entries (storage can hold anything). */
export function parseProbeRecord(value: unknown): ProbeRecord {
  const out: ProbeRecord = {};
  if (typeof value !== 'object' || value === null) return out;
  for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
    const e = entry as Partial<ProbeEntry> | null;
    if (
      e &&
      typeof e.url === 'string' &&
      typeof e.at === 'number' &&
      (e.verdict === 'clean' || e.verdict === 'dirty' || e.verdict === 'unknown')
    ) {
      out[key] = { url: e.url, at: e.at, verdict: e.verdict };
    }
  }
  return out;
}

/** Runs tasks with at most `limit` in flight (for probes started from many places at once). */
export function createLimiter(limit: number) {
  let active = 0;
  const queue: (() => void)[] = [];
  return async <T>(task: () => Promise<T>): Promise<T> => {
    if (active >= limit) await new Promise<void>((resolve) => queue.push(resolve));
    active++;
    try {
      return await task();
    } finally {
      active--;
      queue.shift()?.();
    }
  };
}

/** Runs `task` over `items` with at most `limit` in flight. */
export async function mapLimit<T>(
  items: readonly T[],
  limit: number,
  task: (item: T) => Promise<void>,
): Promise<void> {
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const item = items[next++] as T;
      await task(item);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
}
