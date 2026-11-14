// Settings without the UI (P1.12): the tabs, rule wording, threshold multipliers and
// shortcut recording.
import type { Rule, RuleAction, RuleInput, RuleScope, Thresholds } from '../api';

export const TABS = [
  { id: 'general', label: 'General' },
  { id: 'aging', label: 'Aging' },
  { id: 'protection', label: 'Protection' },
  { id: 'browsers', label: 'Browsers' },
  { id: 'privacy', label: 'Privacy & data' },
  { id: 'shortcuts', label: 'Shortcuts' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'updates', label: 'Updates' },
  { id: 'about', label: 'About' },
] as const;

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
export function thresholdsProblem(t: Thresholds): string | null {
  const vals = [t.stale, t.hidden, t.archive];
  if (vals.some((h) => !Number.isFinite(h) || h < MIN_H || h > MAX_H)) {
    return 'Each threshold is 1 to 720 active hours';
  }
  if (!(t.stale < t.hidden && t.hidden < t.archive)) {
    return 'Stale comes before hidden, and hidden before archived';
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
  if (days < 1) return `≈ ${Math.round(hours)} h of work`;
  if (days < 5) return `≈ ${Math.round(days * 2) / 2} work days`;
  return `≈ ${Math.round((days / 5) * 2) / 2} work weeks`;
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

export const SCOPE_TEXT: Record<RuleScope, string> = {
  domain: 'Site',
  urlGlob: 'Page address',
  titleRegex: 'Title',
  app: 'App',
  global: 'Everything',
};

const ACTION_TEXT: Record<RuleAction, string> = {
  protect: 'Never archive',
  exclude: 'Never store',
  customThreshold: 'Custom timing',
  closeAllowed: 'May close',
};

/** The chip on a rule row. */
export function actionChip(
  rule: Pick<Rule, 'action' | 'thresholdHours'>,
  global: Thresholds,
): string {
  if (rule.action !== 'customThreshold') return ACTION_TEXT[rule.action];
  return intentOf(rule, global) === 'sooner' ? 'Archive sooner' : 'Keep longer';
}

/** The sentence under a rule's pattern ("Archive after 120 active h"). */
export function ruleSentence(rule: Pick<Rule, 'action' | 'thresholdHours' | 'scope'>): string {
  switch (rule.action) {
    case 'protect':
      return 'Never archived';
    case 'exclude':
      return 'Never recorded or stored';
    case 'closeAllowed':
      return 'Windows may be closed, not only hidden';
    default:
      return rule.thresholdHours
        ? `Archived after ${rule.thresholdHours.archive} active h`
        : 'Custom timing (thresholds missing)';
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
