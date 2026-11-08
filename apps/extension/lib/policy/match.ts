import { parse } from 'tldts';
import type { Rule } from '../db/types';

/** What a rule is matched against. */
export interface MatchTarget {
  appId: string;
  url?: string;
  title: string;
}

/** Lowercase hostname of an http(s) URL, else undefined. */
export function hostOf(url: string | undefined): string | undefined {
  if (!url) return undefined;
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return undefined;
    return parsed.hostname.toLowerCase().replace(/\.$/, '');
  } catch {
    return undefined;
  }
}

/**
 * `github.com` matches `github.com` and every subdomain, never `notgithub.com`.
 * IP addresses only match exactly (`1.1` must not match `192.168.1.1`).
 */
export function domainMatches(host: string | undefined, ruleDomain: string): boolean {
  if (!host) return false;
  let d = ruleDomain.toLowerCase().replace(/^\*\./, '').replace(/\.$/, '');
  // Hosts are ASCII (punycode), so a Unicode rule domain (`bücher.de`) is converted the way
  // the browser converts hosts (same steps as decay-core's `domain_matches`).
  if (/[\u0080-\uffff]/.test(d)) {
    try {
      d = new URL(`http://${d}/`).hostname;
    } catch {
      // kept as is: it never matches a real host
    }
  }
  if (!d) return false;
  if (host === d) return true;
  if (parse(host).isIp || parse(d).isIp) return false;
  return host.endsWith(`.${d}`);
}

/**
 * The URL that globs see: userinfo is dropped (`https://github.com@evil.com/` is evil.com),
 * so a glob can't be fooled by it. Non-web URLs are used as-is.
 */
export function globSubject(url: string): string {
  try {
    const u = new URL(url);
    if (u.protocol === 'http:' || u.protocol === 'https:') {
      return `${u.protocol}//${u.host}${u.pathname}${u.search}${u.hash}`;
    }
  } catch {
    // fall through
  }
  return url;
}

/**
 * Wildcard match where only `*` is special (any run of characters); everything else,
 * including `?`, is literal. Case-insensitive, whole string. Iterative with
 * backtracking to the last star only, so it runs in O(pattern × text) at worst —
 * no regex, so user patterns can't cause catastrophic backtracking.
 */
export function globMatches(pattern: string, text: string): boolean {
  const p = pattern.toLowerCase();
  const t = text.toLowerCase();
  let pi = 0;
  let ti = 0;
  let star = -1;
  let mark = 0;
  while (ti < t.length) {
    if (pi < p.length && p[pi] !== '*' && p[pi] === t[ti]) {
      pi++;
      ti++;
    } else if (pi < p.length && p[pi] === '*') {
      star = pi++;
      mark = ti;
    } else if (star !== -1) {
      pi = star + 1;
      ti = ++mark;
    } else {
      return false;
    }
  }
  while (pi < p.length && p[pi] === '*') pi++;
  return pi === p.length;
}

/** Longest title a regex rule is run against. */
export const MAX_TITLE_MATCH_LENGTH = 256;
export const MAX_REGEX_LENGTH = 200;
/**
 * Most backtracking paths a title pattern may have from one start position (see
 * `backtrackingCost`): two unbounded repeats (`.*x.*`).
 */
export const MAX_PATTERN_COST = 256 * 256;
const COST_CAP = 2 ** 32;

/** A choice point's weight (see `backtrackingCost`). */
const choice = (n: number) => (n > 1 ? Math.max(n, 4) : n);

/** `{n}`, `{n,}`, `{n,m}` at `i`: its length and how many repetition counts it allows. */
function bracedFactor(pattern: string, i: number): [number, number] | null {
  const m = /^\{(\d+)(,(\d*))?\}/.exec(pattern.slice(i));
  if (!m) return null;
  if (m[2] === undefined) return [m[0].length, 1];
  if (!m[3]) return [m[0].length, 256];
  const n = Math.min(Number(m[1]), 1_000_000);
  const max = Math.min(Number(m[3]), 1_000_000);
  return [m[0].length, choice(Math.min(Math.max(max - n, 0) + 1, 256))];
}

/**
 * An upper bound on the work a backtracking engine can do from one start position: a
 * sequence multiplies its items' choices, an alternation adds its branches', `{n,m}` is
 * m − n + 1 choices, and `*` / `+` / `{n,}` are up to the title length (256). Every choice
 * point counts at least 4 (`?`, `{1,2}`, a group with alternatives): chains of small
 * choices have long paths (`a?` × 17 takes ~0.4 s at 2¹⁷ paths). Repeated groups and
 * back-references are refused separately, so this bounds the work
 * (`a{1,30}a{1,30}a{1,30}a{1,30}a{1,30}$` has no unbounded repeat but 24 M paths).
 * Same computation as decay-core's `backtracking_cost`.
 */
export function backtrackingCost(pattern: string): number {
  const stack: [number, number][] = [];
  let sum = 0;
  let seq = 1;
  let i = 0;
  while (i < pattern.length) {
    const ch = pattern[i];
    if (ch === '\\') {
      i += 2;
      continue;
    }
    if (ch === '[') {
      i++;
      if (pattern[i] === '^') i++;
      while (i < pattern.length && pattern[i] !== ']') i += pattern[i] === '\\' ? 2 : 1;
      i++;
      continue;
    }
    if (ch === '(') {
      stack.push([sum, seq]);
      sum = 0;
      seq = 1;
      i += pattern[i + 1] === '?' ? 3 : 1;
      continue;
    }
    if (ch === '|') {
      sum = Math.min(sum + seq, COST_CAP);
      seq = 1;
      i++;
      continue;
    }
    if (ch === ')') {
      const inner = Math.min(sum + seq, COST_CAP);
      const outer = stack.pop();
      if (outer) {
        sum = outer[0];
        seq = Math.min(outer[1] * choice(inner), COST_CAP);
      }
      i++;
      continue;
    }
    let factor: [number, number] | null = null;
    if (ch === '?') factor = [1, 4];
    else if (ch === '*' || ch === '+') factor = [1, 256];
    else if (ch === '{') factor = bracedFactor(pattern, i);
    if (factor) {
      seq = Math.min(seq * factor[1], COST_CAP);
      i += factor[0];
      if (pattern[i] === '?') i++; // lazy
    } else {
      i++;
    }
  }
  return Math.min(sum + seq, COST_CAP);
}

/** `(?i)`, `(?i:…)`, `(?-s)` outside an escape (same scan as decay-core's `has_inline_flags`). */
function hasInlineFlags(pattern: string): boolean {
  for (let i = 0; i + 2 < pattern.length; i++) {
    if (pattern[i] === '\\') {
      i++;
      continue;
    }
    if (pattern[i] === '(' && pattern[i + 1] === '?' && /[a-zA-Z-]/.test(pattern[i + 2] ?? '')) {
      return true;
    }
  }
  return false;
}

/**
 * Why a title pattern is unsafe or invalid, or null if it's fine. Patterns run against
 * page titles that websites control, inside the service worker, so anything that can
 * backtrack catastrophically is refused: quantified groups (`(a+)+`), back-references,
 * lookarounds, and patterns with too many backtracking paths (`backtrackingCost`).
 */
export function titleRegexProblem(pattern: string): string | null {
  if (pattern.length > MAX_REGEX_LENGTH) return 'Pattern is too long';
  // Inline modifiers ((?i), (?i:…)) are valid in Chrome 125+ but not everywhere (nor in the
  // desktop engine before this check): refused so a rule never means different things.
  if (hasInlineFlags(pattern)) return 'Inline flags are not supported';
  try {
    new RegExp(pattern, 'i');
  } catch {
    return 'Invalid title pattern';
  }
  let inClass = false;
  for (let i = 0; i < pattern.length; i++) {
    const ch = pattern[i];
    if (ch === '\\') {
      const next = pattern[i + 1] ?? '';
      if (/[1-9k]/.test(next)) return 'Back-references are not supported';
      i++;
      continue;
    }
    if (inClass) {
      if (ch === ']') inClass = false;
      continue;
    }
    if (ch === '[') inClass = true;
    else if (ch === '(' && pattern[i + 1] === '?' && /[=!<]/.test(pattern[i + 2] ?? '')) {
      return 'Lookarounds are not supported';
    } else if (ch === ')' && /[*+?{]/.test(pattern[i + 1] ?? '')) {
      return 'Repeated groups are not supported';
    }
  }
  if (backtrackingCost(pattern) > MAX_PATTERN_COST) return 'Pattern is too complex';
  return null;
}

const regexCache = new Map<string, RegExp | null>();
const CACHE_LIMIT = 5000;

/** Compiles a safe user title pattern; null if invalid or unsafe (then it never matches). */
export function compileTitleRegex(pattern: string): RegExp | null {
  let re = regexCache.get(pattern);
  if (re === undefined) {
    re = titleRegexProblem(pattern) === null ? new RegExp(pattern, 'i') : null;
    if (regexCache.size >= CACHE_LIMIT) regexCache.clear();
    regexCache.set(pattern, re);
  }
  return re;
}

export function ruleMatches(rule: Rule, target: MatchTarget): boolean {
  if (!rule.enabled) return false;
  switch (rule.scope) {
    case 'global':
      return true;
    case 'app':
      return rule.match === target.appId;
    case 'domain':
      return domainMatches(hostOf(target.url), rule.match);
    case 'urlGlob':
      return target.url !== undefined && globMatches(rule.match, globSubject(target.url));
    case 'titleRegex': {
      const re = compileTitleRegex(rule.match);
      return re?.test(target.title.slice(0, MAX_TITLE_MATCH_LENGTH)) === true;
    }
    default:
      return false;
  }
}

/** Most specific first (Docs/Software Design.md §4.1): URL glob > title > domain > app > global. */
export const SCOPE_SPECIFICITY: Record<Rule['scope'], number> = {
  urlGlob: 0,
  titleRegex: 1,
  domain: 2,
  app: 3,
  global: 4,
};
