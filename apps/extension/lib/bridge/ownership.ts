// Who may close tabs: the local engine (standalone) or the Decay app (adapter mode). Never
// both. While the link is on, the local engine waits until a connection attempt has really
// failed: right after a service-worker restart the bridge is still `off` / `connecting`
// although the app may be alive and in charge.

import type { BridgeState } from './connection';

/**
 * Whether the local tracker may plan and execute sweeps now.
 * - Link off: always (Phase 0 behaviour).
 * - Link on: only once an attempt in this worker has ended without a connection
 *   (`unavailable`, `outdated`, or `off` for a missing permission).
 */
export function localEngineMayRun(linkOn: boolean, state: BridgeState, settled: boolean): boolean {
  if (!linkOn) return true;
  if (state === 'connected' || state === 'connecting') return false;
  return settled;
}
