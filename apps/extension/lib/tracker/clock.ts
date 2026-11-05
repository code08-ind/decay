// Aging clock (Docs/Software Design.md §4.2, PRD F1 / story S1 "Monday reset").
// Pure accounting: given what happened since the last tick, how many seconds of
// *active work time* to add to every live item that is not in use.

/** Tick length. `chrome.alarms` allows 30 s periods since Chrome 120. */
export const TICK_MS = 30_000;

/**
 * A gap longer than this between ticks means the machine slept, the browser was
 * closed or the worker was suspended. Such gaps earn no credit (no back-credit):
 * items only ever look *newer* than they are, which is the safe direction.
 */
export const MAX_TICK_GAP_MS = 3 * TICK_MS;

export type IdleState = 'active' | 'idle' | 'locked';

export interface ActivitySample {
  /** Time since the previous tick, or null on the first tick of a browser session. */
  elapsedMs: number | null;
  /** `chrome.idle.queryState(idleCutoffS)`. */
  idleState: IdleState;
  /** Any tab is playing audio (watching a video counts as being active). */
  anyAudible: boolean;
}

/** Seconds of active time to credit for this tick. */
export function computeCredit({ elapsedMs, idleState, anyAudible }: ActivitySample): number {
  if (elapsedMs === null || !(elapsedMs > 0) || elapsedMs > MAX_TICK_GAP_MS) return 0;
  if (idleState === 'locked') return 0;
  if (idleState === 'active' || anyAudible) return Math.round(elapsedMs / 1000);
  return 0;
}
