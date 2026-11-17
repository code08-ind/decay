import Dexie from 'dexie';
import { parse } from 'tldts';
import { uuidv7 } from 'uuidv7';
import { type DecayDB, db as defaultDb } from '../db/db';
import type { Rule, RuleAction, RuleScope, RuleSource, Thresholds } from '../db/types';
import { type MatchTarget, ruleMatches, titleRegexProblem } from '../policy/match';
import { THRESHOLD_MAX_H, THRESHOLD_MIN_H, thresholdsProblem } from '../policy/thresholds';
import { type Key, t } from '../strings';

export interface RuleInput {
  scope: RuleScope;
  match: string;
  action: RuleAction;
  thresholdHours?: Thresholds;
  enabled?: boolean;
  source?: RuleSource;
}

export class RuleError extends Error {
  constructor(
    readonly code: 'INVALID_RULE' | 'DUPLICATE_RULE' | 'NOT_FOUND',
    message: string,
  ) {
    super(message);
    this.name = 'RuleError';
  }
}

const SCOPES: readonly RuleScope[] = ['app', 'domain', 'urlGlob', 'titleRegex', 'global'];
const ACTIONS: readonly RuleAction[] = ['protect', 'exclude', 'customThreshold', 'closeAllowed'];
const SOURCES: readonly RuleSource[] = ['user', 'default', 'quick_action'];
const MAX_MATCH_LENGTH = 2048;

/**
 * Unrelated pages used to catch exclude rules that would match (and purge) almost
 * everything, e.g. `*://*`, `http*`, title `.*`. A real exclude matches at most one.
 */
const BROADNESS_CANARIES: MatchTarget[] = [
  { appId: 'canary', url: 'https://example.com/', title: 'Example Domain' },
  { appId: 'canary', url: 'https://www.wikipedia.org/wiki/Tab', title: 'Tab – Wikipedia' },
  { appId: 'canary', url: 'https://news.ycombinator.com/item?id=1', title: 'Hacker News' },
  { appId: 'canary', url: 'http://localhost:3000/dashboard', title: 'Dashboard · Dev' },
  { appId: 'canary', url: 'https://mail.proton.me/u/0/inbox', title: 'Inbox | Proton Mail' },
];

/**
 * The checks' English words (`lib/policy` returns them, as the desktop engine does) as
 * catalog keys, so the error is in the user's language; anything else as written.
 */
const RULE_TEXT: Record<string, Key> = {
  'Not a valid domain': 'rule.domain',
  'Match is required': 'rule.matchRequired',
  'Match is too long': 'rule.matchLong',
  'Use the global scope to match every URL': 'rule.global',
  'Exclude rules need a domain, URL or title pattern': 'rule.exclude',
  'Close-allowed rules apply to apps only': 'rule.closeAllowed',
  'Custom threshold rules need thresholds': 'rule.thresholds',
  'Pattern is too long': 'rule.titleLong',
  'Inline flags are not supported': 'rule.titleFlags',
  'Invalid title pattern': 'rule.titleInvalid',
  'Back-references are not supported': 'rule.titleBackrefs',
  'Lookarounds are not supported': 'rule.titleLookarounds',
  'Repeated groups are not supported': 'rule.titleRepeats',
  'Pattern is too complex': 'rule.titleComplex',
};

export function ruleText(english: string): string {
  const key = RULE_TEXT[english];
  if (key) return t(key);
  if (english.startsWith('Thresholds must be')) {
    return t('thresholds.range', { min: THRESHOLD_MIN_H, max: THRESHOLD_MAX_H });
  }
  return english;
}

const invalid = (message: string) => new RuleError('INVALID_RULE', ruleText(message));

/**
 * Accepts `github.com`, `*.github.com`, `GitHub.com`, `bücher.de` (→ punycode), an IP,
 * or a pasted URL. Rejects bare public suffixes (`com`, `co.uk`, `github.io`), which
 * would match a huge part of the web.
 */
export function normalizeDomain(input: string): string {
  let value = input.trim().toLowerCase().replace(/^\*\./, '');
  if (!/^[a-z][a-z0-9+.-]*:\/\//.test(value)) value = `http://${value}`;
  let host: string;
  try {
    const url = new URL(value);
    if (url.username || url.password) throw new Error('userinfo');
    host = url.hostname.replace(/\.$/, '');
  } catch {
    throw invalid('Not a valid domain');
  }
  if (!host) throw invalid('Not a valid domain');
  const info = parse(host, { allowPrivateDomains: true });
  if (!info.isIp && host !== 'localhost' && !info.domain) {
    throw new RuleError('INVALID_RULE', t('rule.suffix', { host }));
  }
  return host;
}

/** Validates a rule and returns its normalized fields. */
export function validateRule(input: RuleInput): Required<Omit<RuleInput, 'thresholdHours'>> & {
  thresholdHours?: Thresholds;
} {
  const { scope, action } = input;
  if (!SCOPES.includes(scope)) throw invalid(`Unknown scope: ${String(scope)}`);
  if (!ACTIONS.includes(action)) throw invalid(`Unknown action: ${String(action)}`);
  const source = input.source ?? 'user';
  if (!SOURCES.includes(source)) throw invalid(`Unknown source: ${String(source)}`);

  let match = typeof input.match === 'string' ? input.match.trim() : '';
  if (scope === 'global') match = '*';
  if (!match) throw invalid('Match is required');
  if (match.length > MAX_MATCH_LENGTH) throw invalid('Match is too long');

  if (scope === 'domain') match = normalizeDomain(match);
  if (scope === 'titleRegex') {
    const problem = titleRegexProblem(match);
    if (problem) throw invalid(problem);
  }
  if (scope === 'urlGlob' && /^\**$/.test(match)) {
    throw invalid('Use the global scope to match every URL');
  }

  if (action === 'exclude' && (scope === 'global' || scope === 'app')) {
    throw invalid('Exclude rules need a domain, URL or title pattern');
  }
  if (action === 'closeAllowed' && scope !== 'app') {
    throw invalid('Close-allowed rules apply to apps only');
  }

  let thresholdHours: Thresholds | undefined;
  if (action === 'customThreshold') {
    if (!input.thresholdHours) throw invalid('Custom threshold rules need thresholds');
    const problem = thresholdsProblem(input.thresholdHours);
    if (problem) throw invalid(problem);
    const { stale, hidden, archive } = input.thresholdHours;
    thresholdHours = { stale, hidden, archive };
  }

  if (action === 'exclude') {
    const probe: Rule = {
      id: '',
      scope,
      match,
      action,
      enabled: true,
      source,
      createdAt: 0,
      hitCount: 0,
    };
    const hits = BROADNESS_CANARIES.filter((target) => ruleMatches(probe, target)).length;
    if (hits > 1) throw invalid('This exclude rule matches too many sites; make it more specific');
  }

  return { scope, match, action, enabled: input.enabled ?? true, source, thresholdHours };
}

export function createRulesStore(database: DecayDB = defaultDb) {
  async function list(): Promise<Rule[]> {
    return database.rules.orderBy('scope').toArray();
  }

  /**
   * How many stored items (archives included) an exclude rule would delete — for a
   * confirmation step in the UI before calling `create`/`update`.
   */
  async function previewExclude(input: RuleInput): Promise<number> {
    const fields = validateRule(input);
    if (fields.action !== 'exclude') return 0;
    const probe: Rule = { id: '', ...fields, enabled: true, createdAt: 0, hitCount: 0 };
    return database.items.filter((item) => ruleMatches(probe, item)).count();
  }

  /** Creates a rule. `purged` = items deleted because the rule excludes them (F9). */
  async function create(
    input: RuleInput,
    now = Date.now(),
  ): Promise<{ rule: Rule; purged: number }> {
    const fields = validateRule(input);
    const rule: Rule = { id: uuidv7(), ...fields, createdAt: now, hitCount: 0 };
    if (!rule.thresholdHours) delete rule.thresholdHours;
    let purged = 0;
    await database.transaction('rw', database.rules, database.items, database.events, async () => {
      try {
        await database.rules.add(rule);
      } catch (error) {
        if (error instanceof Dexie.ConstraintError) {
          throw new RuleError('DUPLICATE_RULE', t('rule.duplicate'));
        }
        throw error;
      }
      if (rule.action === 'exclude' && rule.enabled) purged = await purgeExcluded(database, [rule]);
    });
    return { rule, purged };
  }

  async function update(
    id: string,
    patch: Partial<RuleInput>,
  ): Promise<{ rule: Rule; purged: number }> {
    return database.transaction('rw', database.rules, database.items, database.events, async () => {
      const current = await database.rules.get(id);
      if (!current) throw new RuleError('NOT_FOUND', t('rule.notFound'));
      // Disabling must always work, even for a rule that is invalid under today's checks.
      if (Object.keys(patch).every((k) => k === 'enabled') && patch.enabled === false) {
        const next = { ...current, enabled: false };
        await database.rules.put(next);
        return { rule: next, purged: 0 };
      }
      const fields = validateRule({ ...current, ...patch });
      const next: Rule = { ...current, ...fields };
      if (!fields.thresholdHours) delete next.thresholdHours;
      try {
        await database.rules.put(next);
      } catch (error) {
        if (error instanceof Dexie.ConstraintError) {
          throw new RuleError('DUPLICATE_RULE', t('rule.duplicate'));
        }
        throw error;
      }
      const purged =
        next.action === 'exclude' && next.enabled ? await purgeExcluded(database, [next]) : 0;
      return { rule: next, purged };
    });
  }

  async function remove(id: string): Promise<void> {
    await database.rules.delete(id);
  }

  /**
   * "Always keep" a site from a quick action (Insights): a domain protect rule. One already
   * there counts, but turned off it's turned on — a rule that doesn't apply keeps nothing.
   */
  async function alwaysKeep(domain: string): Promise<void> {
    try {
      await create({ scope: 'domain', match: domain, action: 'protect', source: 'quick_action' });
      return;
    } catch (e) {
      if (!(e instanceof RuleError && e.code === 'DUPLICATE_RULE')) throw e;
    }
    const match = normalizeDomain(domain);
    const rule = (await list()).find(
      (r) => r.scope === 'domain' && r.action === 'protect' && r.match === match,
    );
    if (rule && !rule.enabled) await update(rule.id, { enabled: true });
  }

  return { list, previewExclude, create, update, remove, alwaysKeep };
}

/**
 * F9: excluded sites are never stored, "even in events". When an exclude rule is
 * added or enabled, everything already stored for it is deleted, including archives.
 */
export async function purgeExcluded(database: DecayDB, rules: readonly Rule[]): Promise<number> {
  const ids = await database.items
    .filter((item) => rules.some((rule) => ruleMatches(rule, item)))
    .primaryKeys();
  if (ids.length) {
    await database.events.where('itemId').anyOf(ids).delete();
    await database.items.bulkDelete(ids);
  }
  return ids.length;
}

export const rules = createRulesStore();
