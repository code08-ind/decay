import type { Item, Mode, Reason, Rule, Thresholds } from '../db/types';
import { type MatchTarget, ruleMatches, SCOPE_SPECIFICITY } from './match';
import { areValidThresholds } from './thresholds';

/**
 * Policy engine (P0.3, PRD F8/F9; TS port of `Policy::evaluate`, Software Design §4.1).
 * Pure: no I/O. Principle: fail by doing nothing — when in doubt, protect.
 *
 * Order: global hold/pause → hard protections → user rules → thresholds.
 * Rule resolution: any matching `protect` rule wins (safety first); otherwise the most
 * specific `customThreshold` rule replaces the global thresholds (a deliberate
 * deviation from pure "most specific first": protection always beats archiving sooner).
 * `exclude` rules are handled before policy (excluded tabs are never stored, see
 * `isExcluded`); if a site is both protected and excluded, exclude wins — it is never stored.
 */

export interface PolicyContext {
  now: number;
  mode: Mode;
  holdUntil: number | null;
  thresholds: Thresholds;
  rules: readonly Rule[];
}

/** Live signals for a tab, from the browser and the form probe (P0.7). */
export interface TabSignals {
  active: boolean;
  audible: boolean;
  /**
   * Typed input or a "Leave site?" prompt, from the form probe (`lib/probe`).
   * `'unknown'` = the page couldn't be checked: web pages then need twice the archive
   * threshold (PRD §7.3). Omitted = not a concern (non-web pages, tests).
   */
  formInput?: boolean | 'unknown';
}

/** Archive-threshold multiplier when form input can't be detected (PRD §7.3). */
export const UNKNOWN_FORM_FACTOR = 2;

export type PolicyTarget = Pick<
  Item,
  'appId' | 'url' | 'title' | 'pinned' | 'activeAgeS' | 'graceUntil' | 'kind'
>;

export type Decision =
  /** Nothing to do yet. */
  | { action: 'keep'; activeHours: number; threshold: Thresholds }
  | { action: 'protect'; reason: Extract<Reason, { type: 'protected' }> }
  /** Old enough to show as stale (visual hint only). */
  | { action: 'stale'; activeHours: number; threshold: Thresholds; ruleId?: string }
  /** Old enough to archive at the next sweep. */
  | { action: 'archive'; reason: Exclude<Reason, { type: 'protected' }> };

/** Schemes that can be reopened exactly with `tabs.create({ url })`. */
const RESTORABLE = /^(https?:|chrome:|edge:|brave:|chrome-extension:|about:blank([?#].*)?$)/i;

export function isRestorableUrl(url: string | undefined): boolean {
  return url !== undefined && RESTORABLE.test(url);
}

/** Empty new-tab pages: archivable, but not worth showing in search. */
export function isBlankPage(url: string | undefined): boolean {
  return (
    url !== undefined &&
    /^(about:blank|chrome:\/\/newtab\/?|edge:\/\/newtab\/?|brave:\/\/newtab\/?|chrome:\/\/new-tab-page\/?)$/i.test(
      url,
    )
  );
}

/** Excluded (F9): never stored, never touched. */
export function isExcluded(target: MatchTarget, rules: readonly Rule[]): boolean {
  return rules.some((r) => r.action === 'exclude' && ruleMatches(r, target));
}

const protectedBy = (
  by: Extract<Reason, { type: 'protected' }>['by'],
  ruleId?: string,
): Decision => ({
  action: 'protect',
  reason: ruleId ? { type: 'protected', by, ruleId } : { type: 'protected', by },
});

export function evaluate(item: PolicyTarget, signals: TabSignals, ctx: PolicyContext): Decision {
  // 1. Global hold / pause.
  if (ctx.mode === 'paused' || (ctx.holdUntil !== null && ctx.holdUntil > ctx.now)) {
    return protectedBy('hold');
  }

  // 2. Hard protections.
  if (item.pinned) return protectedBy('pinned');
  if (signals.active) return protectedBy('activeTab');
  if (signals.audible) return protectedBy('audible');
  if (signals.formInput === true) return protectedBy('formInput');
  if (item.graceUntil !== undefined && item.graceUntil > ctx.now) return protectedBy('grace');
  if (item.kind === 'tab' && !isRestorableUrl(item.url)) return protectedBy('unrestorable');

  // 3. User rules.
  const matching = ctx.rules
    .filter((r) => r.action === 'protect' || r.action === 'customThreshold')
    .filter((r) => ruleMatches(r, item))
    .sort((a, b) => SCOPE_SPECIFICITY[a.scope] - SCOPE_SPECIFICITY[b.scope]);
  const protect = matching.find((r) => r.action === 'protect');
  if (protect) return protectedBy('rule', protect.id);
  // A custom rule with invalid thresholds (old data, hand edits) is ignored, not obeyed.
  const custom = matching.find(
    (r) => r.action === 'customThreshold' && areValidThresholds(r.thresholdHours),
  );

  // 4. Thresholds.
  const threshold = custom?.thresholdHours ?? ctx.thresholds;
  const activeHours = item.activeAgeS / 3600;
  const shownHours = Math.floor(activeHours);
  // A web page might hold typed input we can't see: archive it only at 2× the threshold.
  const archiveAt =
    signals.formInput === 'unknown' && /^https?:/i.test(item.url ?? '')
      ? threshold.archive * UNKNOWN_FORM_FACTOR
      : threshold.archive;
  if (activeHours >= archiveAt) {
    return {
      action: 'archive',
      reason: custom
        ? { type: 'customRule', ruleId: custom.id, activeHours: shownHours }
        : { type: 'unused', activeHours: shownHours, threshold: archiveAt },
    };
  }
  if (activeHours >= threshold.stale) {
    return custom
      ? { action: 'stale', activeHours, threshold, ruleId: custom.id }
      : { action: 'stale', activeHours, threshold };
  }
  return { action: 'keep', activeHours, threshold };
}

/** The lifecycle state a live tab should show for a decision (tabs skip `hidden`). */
export function stateFor(decision: Decision): 'active' | 'stale' {
  return decision.action === 'stale' || decision.action === 'archive' ? 'stale' : 'active';
}
