import type { Item, ItemEvent, Reason } from '../db/types';
import { type Decision, evaluate, type PolicyContext, stateFor } from '../policy/policy';
import type { TabInfo } from './tabs';

/**
 * Active ⇄ Stale transitions for open tabs, run after each tick's aging
 * (Software Design §4.1 Lifecycle). Archiving itself only happens in sweeps (P0.5).
 * Protected items always show as active. Returns the decision per item id.
 */
export function applyLifecycle(
  items: readonly Item[],
  tabsByExternalId: ReadonlyMap<string, TabInfo>,
  ctx: PolicyContext,
  dirty: Set<Item>,
  events: ItemEvent[],
): Map<string, Decision> {
  const decisions = new Map<string, Decision>();
  for (const item of items) {
    if (item.state !== 'active' && item.state !== 'stale') continue;
    const tab = item.externalId ? tabsByExternalId.get(item.externalId) : undefined;
    const decision = evaluate(
      item,
      { active: tab?.active ?? false, audible: tab?.audible ?? false },
      ctx,
    );
    decisions.set(item.id, decision);
    // A hold or pause is temporary: freeze states instead of flipping every stale tab
    // to active and back (which would also duplicate `stale` events).
    if (decision.action === 'protect' && decision.reason.by === 'hold') continue;
    const next = stateFor(decision);
    if (next === item.state) continue;
    item.state = next;
    item.stateChangedAt = ctx.now;
    dirty.add(item);
    if (next === 'stale') {
      events.push({ itemId: item.id, type: 'stale', at: ctx.now, reason: staleReason(decision) });
    }
  }
  return decisions;
}

function staleReason(decision: Decision): Reason | undefined {
  if (decision.action === 'archive') return decision.reason;
  if (decision.action !== 'stale') return undefined;
  const activeHours = Math.floor(decision.activeHours);
  return decision.ruleId
    ? { type: 'customRule', ruleId: decision.ruleId, activeHours }
    : { type: 'unused', activeHours, threshold: decision.threshold.stale };
}
