// Settings without the UI (P1.12): the tabs, rule wording, threshold multipliers and
// shortcut recording.
import type { Rule, RuleAction, RuleInput, RuleScope, Thresholds } from '../api';
import { type Key, t } from '../strings';

export const TABS = [
  { id: 'general', label: 'settings.tab.general' },
  { id: 'aging', label: 'settings.tab.aging' },
  { id: 'protection', label: 'settings.tab.protection' },
  { id: 'browsers', label: 'settings.tab.browsers' },
  { id: 'privacy', label: 'settings.tab.privacy' },
  { id: 'shortcuts', label: 'settings.tab.shortcuts' },
  { id: 'notifications', label: 'settings.tab.notifications' },
  { id: 'updates', label: 'settings.tab.updates' },
  { id: 'about', label: 'settings.tab.about' },
] as const satisfies readonly { id: string; label: Key }[];

export type TabId = (typeof TABS)[number]['id'];

/** The tab a route opens (`/aging`, deep links `decay://settings/{tab}`). Tabs not built
 * yet (license) open General. */
export function tabFor(route: string): TabId {
  const name = route.replace(/^\//, '').split(/[/?]/)[0] ?? '';
  return (TABS.find((t) => t.id === name)?.id as TabId | undefined) ?? 'general';
}

export const BALANCED: Thresholds = { stale: 12, hidden: 36, archive: 60 };
export const PRESET_HOURS: Record<'gentle' | 'balanced' | 'aggressive', Thresholds> = {
  gentle: { stale: 24, hidden: 60, archive: 96 },
  balanced: BALANCED,
  aggressive: { stale: 8, hidden: 24, archive: 40 },
};

const MIN_H = 1;
const MAX_H = 720;

/** Thresholds valid for the core: each 1–720 h, strictly increasing. */
export function thresholdsProblem(h: Thresholds): string | null {
  const vals = [h.stale, h.hidden, h.archive];
  if (vals.some((h) => !Number.isFinite(h) || h < MIN_H || h > MAX_H)) {
    return t('thresholds.range');
  }
  if (!(h.stale < h.hidden && h.hidden < h.archive)) {
    return t('thresholds.order');
  }
  return null;
}

/** The pace the thresholds match, or `custom`. */
export function paceOf(t: Thresholds | null | undefined): keyof typeof PRESET_HOURS | 'custom' {
  if (!t) return 'balanced';
  for (const [name, p] of Object.entries(PRESET_HOURS)) {
    if (p.stale === t.stale && p.hidden === t.hidden && p.archive === t.archive) {
      return name as keyof typeof PRESET_HOURS;
    }
  }
  return 'custom';
}

/** "≈ 1.5 work days" (8 active hours a day; 5 days a week). */
export function workTime(hours: number): string {
  const days = hours / 8;
  if (days < 1) return t('workTime.hours', { n: Math.round(hours) });
  if (days < 5) return t('workTime.days', { n: Math.round(days * 2) / 2 });
  return t('workTime.weeks', { n: Math.round((days / 5) * 2) / 2 });
}

/**
 * A rule's thresholds as a multiple of the global ones (Designs/08 "Keep longer 2×" /
 * "Archive sooner"): each scaled, rounded to whole hours, kept in 1–720 and increasing.
 */
export function scaled(global: Thresholds, factor: number): Thresholds {
  const clamp = (h: number) => Math.min(MAX_H, Math.max(MIN_H, Math.round(h)));
  const stale = clamp(global.stale * factor);
  const hidden = Math.max(stale + 1, clamp(global.hidden * factor));
  const archive = Math.max(hidden + 1, clamp(global.archive * factor));
  return {
    stale: Math.min(stale, MAX_H - 2),
    hidden: Math.min(hidden, MAX_H - 1),
    archive: Math.min(archive, MAX_H),
  };
}

/** The factor a custom threshold rule was made with (archive hours / global archive). */
export function factorOf(rule: Pick<Rule, 'thresholdHours'>, global: Thresholds): number {
  const a = rule.thresholdHours?.archive;
  if (!a || global.archive <= 0) return 1;
  return Math.round((a / global.archive) * 4) / 4;
}

/** The editor slider's starting value for a timing rule: 1.25–5 either way. */
export function sliderFactor(rule: Pick<Rule, 'thresholdHours'>, global: Thresholds): number {
  const f = factorOf(rule, global);
  const v = f >= 1 ? f : f > 0 ? Math.round((1 / f) * 4) / 4 : 5;
  return Math.min(5, Math.max(1.25, v));
}

export type Intent = 'protect' | 'longer' | 'sooner' | 'exclude';

/** What a rule does, in the editor's words. */
export function intentOf(
  rule: Pick<Rule, 'action' | 'thresholdHours'>,
  global: Thresholds,
): Intent {
  if (rule.action === 'exclude') return 'exclude';
  if (rule.action === 'customThreshold') return factorOf(rule, global) >= 1 ? 'longer' : 'sooner';
  return 'protect';
}

/** A scope's name ("Site"). */
export const scopeText = (scope: RuleScope): string => t(`rule.scope.${scope}`);

const actionText = (action: RuleAction): string => t(`rule.action.${action}`);

/** The chip on a rule row. */
export function actionChip(
  rule: Pick<Rule, 'action' | 'thresholdHours'>,
  global: Thresholds,
): string {
  if (rule.action !== 'customThreshold') return actionText(rule.action);
  return intentOf(rule, global) === 'sooner' ? t('rule.chip.sooner') : t('rule.chip.longer');
}

/** The sentence under a rule's pattern ("Archive after 120 active h"). */
export function ruleSentence(rule: Pick<Rule, 'action' | 'thresholdHours' | 'scope'>): string {
  switch (rule.action) {
    case 'protect':
      return t('rule.sentence.protect');
    case 'exclude':
      return t('rule.sentence.exclude');
    case 'closeAllowed':
      return t('rule.sentence.closeAllowed');
    default:
      return rule.thresholdHours
        ? t('rule.sentence.after', { n: rule.thresholdHours.archive })
        : t('rule.sentence.missing');
  }
}

/** The input for saving an editor's state. */
export function ruleInput(
  scope: RuleScope,
  match: string,
  intent: Intent,
  factor: number,
  global: Thresholds,
  enabled = true,
): RuleInput {
  if (intent === 'protect') return { scope, match, action: 'protect', enabled };
  if (intent === 'exclude') return { scope, match, action: 'exclude', enabled };
  const f = intent === 'longer' ? factor : 1 / factor;
  return { scope, match, action: 'customThreshold', thresholdHours: scaled(global, f), enabled };
}

/** Placeholder for the match field of a scope. */
export function matchHint(scope: RuleScope): string {
  switch (scope) {
    case 'domain':
      return 'mail.google.com';
    case 'urlGlob':
      return 'github.com/*/pull/*';
    case 'titleRegex':
      return 'standup|retro';
    default:
      return '';
  }
}

const KEY_NAMES: Record<string, string> = {
  ' ': 'Space',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
  Escape: 'Escape',
  Enter: 'Enter',
  Backspace: 'Backspace',
  Tab: 'Tab',
};

/**
 * A global shortcut from a key press, as an accelerator ("Alt+Super+D"); `null` while
 * only modifiers are held, or for a key without a modifier (a global shortcut needs one).
 */
export function acceleratorFrom(e: {
  key: string;
  code?: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): string | null {
  if (['Alt', 'Control', 'Meta', 'Shift', 'OS', 'Hyper', 'Super'].includes(e.key)) return null;
  // The physical key: Option+D types "∂" on a Mac.
  let key: string | undefined;
  if (e.code?.startsWith('Key')) key = e.code.slice(3);
  else if (e.code?.startsWith('Digit')) key = e.code.slice(5);
  else if (/^F\d{1,2}$/.test(e.key)) key = e.key;
  else key = KEY_NAMES[e.key] ?? (e.key.length === 1 ? e.key.toUpperCase() : undefined);
  if (!key) return null;
  const mods = [
    e.ctrlKey && 'Ctrl',
    e.altKey && 'Alt',
    e.shiftKey && 'Shift',
    e.metaKey && 'Super',
  ].filter(Boolean) as string[];
  if (mods.length === 0 || (mods.length === 1 && mods[0] === 'Shift')) return null;
  return [...mods, key].join('+');
}
