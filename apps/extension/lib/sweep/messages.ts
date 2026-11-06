import { browser } from 'wxt/browser';

/**
 * Messages from extension pages (popup, Sweeps page) to the background worker, which owns
 * the tracker: sweeps and undo must be serialized with ticks, so pages never run them.
 */
export type DecayRequest =
  | { type: 'decay.sweepNow' }
  | { type: 'decay.undoSweep'; sweepId?: string };

export type DecayResponse =
  | { ok: true; archived?: number; failed?: number; restored?: number }
  | { ok: false; error: string };

export function isDecayRequest(msg: unknown): msg is DecayRequest {
  if (!msg || typeof msg !== 'object') return false;
  const m = msg as { type?: unknown; sweepId?: unknown };
  if (m.type === 'decay.sweepNow') return true;
  return m.type === 'decay.undoSweep' && (m.sweepId === undefined || typeof m.sweepId === 'string');
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
