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
  const d = ruleDomain.toLowerCase().replace(/^\*\./, '').replace(/\.$/, '');
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
/** Unbounded quantifiers allowed in one title pattern (bounds worst-case time). */
const MAX_UNBOUNDED_QUANTIFIERS = 2;

/**
 * Why a title pattern is unsafe or invalid, or null if it's fine. Patterns run against
 * page titles that websites control, inside the service worker, so anything that can
 * backtrack catastrophically is refused: quantified groups (`(a+)+`), back-references,
 * lookarounds, and more than two unbounded quantifiers.
 */
export function titleRegexProblem(pattern: string): string | null {
  if (pattern.length > MAX_REGEX_LENGTH) return 'Pattern is too long';
  try {
    new RegExp(pattern, 'i');
  } catch {
    return 'Invalid title pattern';
  }
  let unbounded = 0;
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
    } else if (ch === '*' || ch === '+') unbounded++;
    else if (ch === '{' && /^\{\d*,\}/.test(pattern.slice(i))) unbounded++;
  }
  if (unbounded > MAX_UNBOUNDED_QUANTIFIERS) return 'Pattern is too complex';
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
