import { browser } from 'wxt/browser';

/**
 * Messages from extension pages (popup, Sweeps page) to the background worker, which owns
 * the tracker: sweeps, undo and restores must be serialized with ticks, so pages never
 * run them.
 */
export type DecayRequest =
  | { type: 'decay.sweepNow' }
  | { type: 'decay.undoSweep'; sweepId?: string }
  /** Restore items; with `intoWindowId` they open there (the first one active). */
  | { type: 'decay.restore'; ids: string[]; intoWindowId?: number }
  /** Restore the whole batch an item was archived with (PRD F6, ⌘/Ctrl+Enter). */
  | { type: 'decay.restoreBatch'; itemId: string };

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
        (m.intoWindowId === undefined || Number.isInteger(m.intoWindowId))
      );
    case 'decay.restoreBatch':
      return isString(m.itemId);
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
