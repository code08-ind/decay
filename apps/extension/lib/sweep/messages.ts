import { browser } from 'wxt/browser';

/**
 * Messages from extension pages (popup, Sweeps page) to the background worker, which owns
 * the tracker: sweeps, undo and restores must be serialized with ticks, so pages never
 * run them.
 */
export type DecayRequest =
  | { type: 'decay.sweepNow' }
  | { type: 'decay.undoSweep'; sweepId?: string }
  /**
   * Restore items; with `intoWindowId` they open there (the first one active). `searchMs`:
   * popup search → restore time (P0.8). The worker records it, because restoring an active
   * tab closes the popup before it could write anything after the reply.
   */
  | { type: 'decay.restore'; ids: string[]; intoWindowId?: number; searchMs?: number }
  /** Restore the whole batch an item was archived with (PRD F6, ⌘/Ctrl+Enter). */
  | { type: 'decay.restoreBatch'; itemId: string; searchMs?: number };

export type DecayResponse =
  | {
      ok: true;
      archived?: number;
      failed?: number;
      restored?: number;
      /** Requested items that were already open again. */
      skipped?: number;
      /** Shadow mode "Sweep now": items the simulation flagged. Nothing was closed. */
      simulated?: number;
    }
  | { ok: false; error: string };

const isString = (v: unknown): v is string => typeof v === 'string' && v.length > 0;
const isMs = (v: unknown) =>
  v === undefined || (typeof v === 'number' && Number.isFinite(v) && v >= 0);

export function isDecayRequest(msg: unknown): msg is DecayRequest {
  if (!msg || typeof msg !== 'object') return false;
  const m = msg as Record<string, unknown>;
  switch (m.type) {
    case 'decay.sweepNow':
      return true;
    case 'decay.undoSweep':
      return m.sweepId === undefined || isString(m.sweepId);
    case 'decay.restore':
      return (
        Array.isArray(m.ids) &&
        m.ids.length > 0 &&
        m.ids.length <= 500 &&
        m.ids.every(isString) &&
        (m.intoWindowId === undefined || Number.isInteger(m.intoWindowId)) &&
        isMs(m.searchMs)
      );
    case 'decay.restoreBatch':
      return isString(m.itemId) && isMs(m.searchMs);
    default:
      return false;
  }
}

/** Sends a request to the worker; rejects with the worker's error message. */
export async function sendDecay(
  request: DecayRequest,
): Promise<Extract<DecayResponse, { ok: true }>> {
  let response: DecayResponse | undefined;
  try {
    response = (await browser.runtime.sendMessage(request)) as DecayResponse | undefined;
  } catch {
    // The worker restarted mid-request ("message channel closed", "Receiving end does not exist").
    response = undefined;
  }
  if (!response) throw new Error('Decay is not responding. Try again in a moment.');
  if (!response.ok) throw new Error(response.error);
  return response;
}
