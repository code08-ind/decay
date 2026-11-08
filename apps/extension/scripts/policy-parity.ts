// Generates the Rust parity fixtures from the extension's real policy code:
//   crates/decay-core/tests/fixtures/policy_parity.json — `evaluate()` over seeded tab cases
//   crates/decay-core/tests/fixtures/title_parity.json  — `titleRegexProblem()` and title
//     matching over pattern × title pairs
//   crates/decay-core/tests/fixtures/fold_parity.json   — case-insensitive matching of every
//     cased BMP code unit, and of random class ranges
//   crates/decay-core/src/js_case.rs — JS's `Canonicalize` and `\s` tables, which decay-core
//     uses to match title patterns exactly like `new RegExp(p, 'i')`
// The Rust tests (`tests/policy_parity.rs`) assert that decay-core decides the same for
// every case (Software Design §4.1: the two policies must stay identical). CI regenerates
// all of them and fails on any diff. Re-run after changing either policy:
//   pnpm --filter @decay/extension parity
// The tables and regex results come from V8 / ICU, so run it with the Node version in
// .nvmrc: another Unicode version can change them.
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import type { Mode, Rule, Thresholds } from '../lib/db/types';
import { ruleMatches, titleRegexProblem } from '../lib/policy/match';
import {
  evaluate,
  isBlankPage,
  isExcluded,
  type PolicyContext,
  type PolicyTarget,
  type TabSignals,
} from '../lib/policy/policy';

// mulberry32: a full-period 32-bit PRNG (the old LCG lost precision and cycled early).
let state = 0x5eed;
const rand = () => {
  state = (state + 0x6d2b79f5) | 0;
  let x = Math.imul(state ^ (state >>> 15), 1 | state);
  x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x;
  return ((x ^ (x >>> 14)) >>> 0) / 2 ** 32;
};
const pick = <T>(xs: readonly T[]): T => xs[Math.floor(rand() * xs.length)] as T;
const shuffle = <T>(xs: T[]): T[] => {
  for (let i = xs.length - 1; i > 0; i--) {
    const j = Math.floor(rand() * (i + 1));
    [xs[i], xs[j]] = [xs[j] as T, xs[i] as T];
  }
  return xs;
};

const URLS = [
  'https://example.com/a',
  'https://docs.github.com/x',
  'https://gist.github.com/1',
  'https://notgithub.com/',
  'http://192.168.1.1/',
  'chrome://newtab/',
  'chrome://settings',
  'data:,x',
  'about:blank',
  'about:blank#x',
  'https://github.com@evil.com/',
  'file:///a.pdf',
  undefined,
  'https://Mail.Google.com./inbox',
  'https://example.com:8443/p?q=1#h',
  'javascript:1',
  'https://acme.notion.so/page',
  'edge://newtab/',
  'https://example.com/b',
  'https://www.figma.com/file/1',
  'http://example.com/',
  'https://bücher.de/katalog',
  'https://xn--bcher-kva.de/',
  'http://[::1]:8080/x',
  'HTTPS://EXAMPLE.COM/UPPER',
  'https://example.com/?',
  'https://example.com/#',
  'https://example.com/a%20b?x=%41',
  'http://127.1/',
  'https://sub.domain.example.com./deep/path',
  'brave://newtab',
  'brave://settings',
  'chrome-extension://abcdefgh/page.html',
  'about:blankx',
  'about:blank?x=1',
  'ABOUT:BLANK',
  'chrome://new-tab-page/',
  'http://10.0.0.1/',
  'https://example.com/Path#Frag',
  'https://a.b.example.com/x#frag',
  'https://kakao.com/',
];
const TITLES = [
  'Inbox (3) unread',
  'Design — Figma',
  '',
  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!',
  'Q3 planning notes',
  '{draft} Q3 plan',
  'le café! — Résumé',
  'Σίσυφος ٣ ١٢٣',
  '😀😀 party',
  `${'𝒜'.repeat(127)}needle`,
  `${'x'.repeat(300)}needle`,
  'Gelen Kutusu – Yazı',
  'ſecret',
];
const th = (stale: number, hidden: number, archive: number): Thresholds => ({
  stale,
  hidden,
  archive,
});
let n = 0;
const r = (over: Partial<Rule>): Rule => ({
  id: `r${n++}`,
  scope: 'domain',
  match: 'example.com',
  action: 'protect',
  enabled: true,
  source: 'user',
  createdAt: 0,
  hitCount: 0,
  ...over,
});
const POOL: Rule[] = [
  r({ scope: 'domain', match: 'github.com' }),
  r({ scope: 'urlGlob', match: 'https://example.com/*' }),
  r({
    scope: 'domain',
    match: 'example.com',
    action: 'customThreshold',
    thresholdHours: th(1, 2, 3),
  }),
  r({
    scope: 'urlGlob',
    match: 'https://docs.github.com/*',
    action: 'customThreshold',
    thresholdHours: th(20, 30, 40),
  }),
  r({
    scope: 'domain',
    match: 'notion.so',
    action: 'customThreshold',
    thresholdHours: th(0, 0, 0),
  }),
  r({ scope: 'titleRegex', match: '^Inbox' }),
  r({ scope: 'titleRegex', match: '(a+)+$' }),
  r({
    scope: 'titleRegex',
    match: 'figma$',
    action: 'customThreshold',
    thresholdHours: th(2, 4, 8),
  }),
  r({
    scope: 'app',
    match: 'com.google.Chrome',
    action: 'customThreshold',
    thresholdHours: th(5, 10, 20),
  }),
  r({ scope: 'domain', match: 'google.com', enabled: false }),
  r({ scope: 'domain', match: 'google.com', action: 'exclude' }),
  r({ scope: 'global', match: '*', action: 'customThreshold', thresholdHours: th(6, 18, 30) }),
  r({ scope: 'app', match: 'com.google.Chrome', action: 'closeAllowed' }),
  r({ scope: 'domain', match: '192.168.1.1' }),
  r({ scope: 'urlGlob', match: 'https://github.com*' }),
  r({ scope: 'titleRegex', match: '\\w{3,30}' }),
  r({ scope: 'titleRegex', match: '{draft}' }),
  r({ scope: 'titleRegex', match: '\\bcafé\\b' }),
  r({ scope: 'titleRegex', match: 'needle' }),
  r({
    scope: 'titleRegex',
    match: '^\\d',
    action: 'customThreshold',
    thresholdHours: th(1.5, 2.5, 3.5),
  }),
  r({ scope: 'domain', match: 'bücher.de' }),
  r({ scope: 'domain', match: '*.domain.example.com.' }),
  r({
    scope: 'urlGlob',
    match: 'https://example.com/*',
    action: 'customThreshold',
    thresholdHours: th(1, 2, 720),
  }),
  r({ scope: 'titleRegex', match: 'yazi' }),
  r({ scope: 'domain', match: '1.1' }),
  r({ scope: 'domain', match: '0.0.1' }),
  r({ scope: 'domain', match: '\u212Aakao.com' }), // Kelvin sign
  r({ scope: 'urlGlob', match: 'HTTPS://EXAMPLE.COM/P*' }),
  r({ scope: 'urlGlob', match: '*#frag' }),
  r({ scope: 'urlGlob', match: 'https://*a*b*example.com/*' }),
  r({ scope: 'global', match: '*', action: 'protect', id: '' }),
  r({ scope: 'titleRegex', match: '[ą-ż]' }),
  r({ scope: 'titleRegex', match: '\\b+' }),
  r({ scope: 'titleRegex', match: 'a{,}' }),
  r({ scope: 'domain', match: 'xn--bcher-kva.de', action: 'exclude' }),
  r({ scope: 'titleRegex', match: 'Q3', action: 'exclude' }),
  r({
    scope: 'domain',
    match: 'example.com',
    action: 'customThreshold',
    thresholdHours: th(2, 1, 3),
  }),
  r({
    scope: 'urlGlob',
    match: '*',
    action: 'customThreshold',
    thresholdHours: th(0.5, 1, 2),
  }),
  r({
    scope: 'titleRegex',
    match: '.',
    action: 'customThreshold',
    thresholdHours: th(1, 2, 3),
  }),
];
const FORM: TabSignals['formInput'][] = [
  undefined,
  undefined,
  undefined,
  true,
  false,
  false,
  'unknown',
  'unknown',
];
const PRESETS = [th(12, 36, 60), th(24, 60, 96), th(8, 24, 40), th(1, 2, 720), th(0.5, 1.5, 2.5)];
const NOW = 1_000_000;

/** Ages clustered at thresholds (exact, ±1 s, 2×) plus uniform ones. */
function age(t: Thresholds): number {
  const edges = [t.stale, t.archive, t.archive * 2, 3, 40, 60, 120];
  if (rand() < 0.5) return Math.floor(rand() * 150 * 3600);
  return Math.max(0, Math.round(pick(edges) * 3600) + pick([-1, 0, 0, 1]));
}

const cases = [];
for (let i = 0; i < 6000; i++) {
  const ruleIdx = shuffle(POOL.map((_, k) => k).filter(() => rand() < 0.08));
  const rules = ruleIdx.map((k) => POOL[k] as Rule);
  const thresholds = pick(PRESETS);
  const ctx: PolicyContext = {
    now: NOW,
    mode: pick<Mode>([
      'automatic',
      'automatic',
      'automatic',
      'shadow',
      'shadow',
      'shadow',
      'automatic',
      'automatic',
      'automatic',
      'paused',
    ]),
    holdUntil: pick([null, null, null, null, null, null, null, null, 500, NOW, 2_000_000]),
    thresholds,
    rules,
  };
  const item: PolicyTarget = {
    kind: 'tab',
    appId: pick(['com.google.Chrome', 'com.microsoft.Edge']),
    url: pick(URLS),
    title: pick(TITLES),
    pinned: rand() < 0.03,
    activeAgeS: age(thresholds),
    graceUntil: pick([undefined, undefined, undefined, undefined, undefined, 500, NOW, 2_000_000]),
  };
  const signals: TabSignals = {
    active: rand() < 0.03,
    audible: rand() < 0.03,
    formInput: pick(FORM),
  };
  const { rules: _omit, ...ctxOut } = ctx;
  cases.push({
    ctx: ctxOut,
    ruleIdx,
    item,
    signals,
    decision: evaluate(item, signals, ctx),
    excluded: isExcluded(item, rules),
    blank: isBlankPage(item.url),
  });
}

// Title patterns: validity and matching, engine-for-engine.
const PATTERNS = [
  '\\w{3,30}',
  '\\w{50}\\d{50}',
  '[\\w\\s]{1,60}',
  '{draft}',
  'a{,5}',
  'a{b',
  '{',
  'a{1,2,3}',
  '[^]',
  '\\0',
  '\\cA',
  '\\e',
  '\\q',
  '\\Z',
  '\\Qa\\E',
  '[\\d-z]',
  '[a-\\d]',
  'a{99999}',
  '\\a',
  '\\z',
  '\\A',
  '\\p{L}',
  '\\pL',
  '\\u{41}',
  '\\u0041',
  '\\x41',
  '[[:alpha:]]',
  '[]a]',
  '[^]a]',
  '[a&&b]',
  '\\d',
  '\\w+',
  '\\bcafé\\b',
  '^.$',
  '^\\S$',
  '[^\\W]',
  'ſ',
  'k',
  'secret',
  '^[a-z]+$',
  '^😀{2}',
  '😀',
  '\\😀',
  '(?i)abc',
  '(?s).x',
  '(?P<n>a)',
  'a**',
  'a++',
  'a{2}{3}',
  '[a--b]',
  '(?i:x)',
  '^Inbox',
  'jira|confluence',
  '\\(\\d+\\) unread',
  '.*bank.*',
  '[a-z]{2,5}',
  '(a+)+$',
  '(a|aa)*',
  '(.*)*',
  '(a)\\1',
  '(?=a)a',
  '(?<n>a)',
  '.*.*.*x',
  'needle',
  'yazi',
  'yazı',
  'Résumé',
  'RÉSUMÉ',
  'σίσυφος',
  '^$',
  '',
  'figma$',
  '\\.',
  'a\\b',
  '[\\u0000-\\u007f]+$',
  'é'.repeat(201),
  '😀'.repeat(101),
  'x'.repeat(200),
  '\\',
  '(',
  ')',
  '[',
  '*',
  '+x',
  '?',
  '[^a-z]',
  '[^\\W]',
  '\\W',
  '[\\W]',
  '\\\\(?i)',
  '(?:x)',
  '(?-i)x',
  'a\\u017F',
  '\\u{1F600}',
  '[😀]',
  '[^😀]',
  'K',
  'Ω',
  'Å',
  'ẞ',
  'ı',
  'ω',
  'å',
  'ß',
  '\\uD83D\\uDE00',
  '\\u00e9',
  '[\\u0130]',
  '[^\\W\\d]',
  '\\u',
  '[😀-😁]',
  '[a-😀]',
  '😀{2}',
  '^😀+$',
  '(?:😀)+',
  '[😀]+x',
  '[\\uD83D\\uDE00]',
  '\\uD83D\\uDE00{2}',
  // Quantified assertions, braces and counts (QA P1.1).
  '\\b+',
  'x\\B?',
  '\\b{2}',
  '\\B*?',
  '\\b{',
  '\\b{2',
  '^*',
  '$+',
  '^{2}',
  '{2}',
  '|{2}',
  '({2})',
  'x{2,1}',
  'x{1,2}',
  'a{1,}',
  'a{,}',
  'a{4294967296}',
  'a{2,99999999999999999999}',
  '\\8\\b+',
  '\\k\\b+',
  '\\k',
  '(?<n>a)\\k',
  // Escapes (Annex B).
  '[\\c]',
  '[\\c1]',
  '[\\c_]',
  '\\c',
  '\\c1',
  '\\cj',
  '[\\377]',
  '[\\400]',
  '[\\08]',
  '\\08',
  '\\07',
  '\\037',
  '[\\b]',
  '[\\B]',
  '[\\x-a]',
  '[a-\\x]',
  '[\\-a]',
  '[--a]',
  '[a-]',
  '\\x4',
  '\\x4a',
  '[\\u12]',
  '[\\u004a-\\u004c]',
  '\\',
  '[]',
  '[]a]',
  '[^]',
  '[a-😀]',
  '[\\d-\\w]',
  // Ranges that contain characters JS doesn't fold (review / QA P1.1).
  '[ą-ż]',
  '[ı-ł]',
  '[Ā-ž]',
  '[^ą-ż]',
  '[Ā-ɏ]',
  '[Ā-ɏ]+',
  '[℀-⅏]',
  '[^\\u0080-\\uffff]',
  '[^|-ƫ]',
  '[ſ-ƀ]',
  '[K-ⅼ]',
  '[İ-Ĳ]',
  '[Ẑ-Ạ]',
  '[\\W]',
  '[^\\S]',
  // Characters JS doesn't fold that other engines do.
  'ᾆ',
  'ᾎ',
  '[ϴ]',
  '[θ]',
  '[ϑ]',
  'Θ',
  'ﬅ',
  '[ﬅ]',
  '[ΐ]',
  '[ΰ]',
  'µ',
  '[µ]',
  // Backtracking budget (QA P1.1).
  'a{1,30}a{1,30}a{1,30}a{1,30}a{1,30}$',
  'a{1,30}a{1,30}a{1,30}$',
  '\\w{1,40}\\w{1,40}\\w{1,40}\\w{1,40}\\w{1,40}!',
  '.*x.*y?',
  '.*x.*y?z?',
  'a?a?a?a?a?a?a?a?a?a?a?a?a?a?a?a?a?a?aaaaaaaaaaaaaaaaaa',
  '(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)(a|a)b',
  '(?:a|b|c)+',
  'a+?b*?',
  'jira|confluence|github|gitlab|notion|figma|slack|linear',
];
const MORE_TITLES = [
  ...TITLES,
  'é',
  '𝒜',
  '\r',
  '٣',
  'S',
  'K',
  'ſecret',
  'ABC',
  'p{L}',
  'a',
  '\u0001',
  '\u0000',
  '-',
  'q',
  'x',
  'yazı',
  'yazi',
  'RÉSUMÉ',
  'résumé',
  'Ⅻ',
  'Ǆ',
  'ω',
  'å',
  'ß',
  'SS',
  'Ω',
  'Å',
  'K',
  'ẞ',
  'İ',
  'u'.repeat(41),
  '\\(?i)',
  '9',
  'Gelen Kutusu – Yazı',
  'żółw',
  'Łódź',
  'k',
  's',
  'Straße',
  'ı',
  'ǅ',
  'θ',
  'ϑ',
  'ϴ',
  'Θ',
  'ᾆ',
  'ᾎ',
  'ﬅ',
  'ﬆ',
  'ΐ',
  'ΐ',
  'µ',
  'Μ',
  'μ',
  '\\',
  'c',
  '\u0011',
  '\u001f',
  '\u0007',
  '\u0008',
  'ÿ',
  'Ÿ',
  ' 0',
  '\u00008',
  '8',
  'J',
  'jkl',
  '{',
  '{2}',
  '|',
  'xx',
  'xxx',
  `${'a'.repeat(200)}b`,
  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab',
  'Research topic',
];
const titleCases = [];
for (const pattern of PATTERNS) {
  const problem = titleRegexProblem(pattern);
  const rule = r({ scope: 'titleRegex', match: pattern });
  for (const title of MORE_TITLES) {
    titleCases.push({
      pattern,
      problem,
      title,
      matches: ruleMatches(rule, { appId: 'x', url: undefined, title }),
    });
  }
}

// Every cased BMP code unit, as a literal and in classes, against its case partners; plus
// random class ranges against characters JS folds unusually.
const hex4 = (u: number) => u.toString(16).padStart(4, '0');
const cased: number[] = [];
for (let u = 0; u < 0x10000; u++) {
  const s = String.fromCharCode(u);
  if (s.toUpperCase() !== s || s.toLowerCase() !== s) cased.push(u);
}
const foldCases: { p: string[]; t: string[]; m: number[][] }[] = [];
for (const u of cased) {
  const s = String.fromCharCode(u);
  const titles = [...new Set([s, s.toUpperCase(), s.toLowerCase()])].filter((t) => t.length === 1);
  const patterns = [`\\u${hex4(u)}`, `[\\u${hex4(u)}]`, `[^\\u${hex4(u)}]`];
  const rules = patterns.map((match) => r({ scope: 'titleRegex', match }));
  foldCases.push({
    p: patterns,
    t: titles,
    m: rules.map((rule) =>
      titles.map((title) => (ruleMatches(rule, { appId: 'x', url: undefined, title }) ? 1 : 0)),
    ),
  });
}
const ODD = [
  0x49, 0x4b, 0x53, 0x69, 0x6b, 0x73, 0xb5, 0xdf, 0x130, 0x131, 0x17f, 0x1c5, 0x345, 0x398, 0x3b8,
  0x3d1, 0x3f4, 0x390, 0x1e9e, 0x1fd3, 0x2126, 0x212a, 0x212b, 0x3a9, 0x3c9, 0xe5, 0xc5, 0xfb05,
  0xfb06,
].map((u) => String.fromCharCode(u));
for (let k = 0; k < 400; k++) {
  const zone = pick([
    [0x41, 0x7a],
    [0x80, 0x250],
    [0x370, 0x400],
    [0x1e00, 0x1fff],
    [0x2100, 0x2200],
    [0xfb00, 0xfb10],
  ] as const);
  const a = zone[0] + Math.floor(rand() * (zone[1] - zone[0]));
  const b = a + Math.floor(rand() * Math.min(200, zone[1] - a + 1));
  const patterns = [`[\\u${hex4(a)}-\\u${hex4(b)}]`, `[^\\u${hex4(a)}-\\u${hex4(b)}]`];
  const rules = patterns.map((match) => r({ scope: 'titleRegex', match }));
  const titles = [...ODD, String.fromCharCode(a), String.fromCharCode(b)];
  foldCases.push({
    p: patterns,
    t: titles,
    m: rules.map((rule) =>
      titles.map((title) => (ruleMatches(rule, { appId: 'x', url: undefined, title }) ? 1 : 0)),
    ),
  });
}

// JS `Canonicalize` (non-u, i) and the `\s` set, for decay-core's title-pattern engine.
const canonical: string[] = [];
for (let u = 0; u < 0x10000; u++) {
  const up = String.fromCharCode(u).toUpperCase();
  if (up.length !== 1) continue;
  const cu = up.charCodeAt(0);
  if (cu === u || (u >= 128 && cu < 128)) continue;
  canonical.push(`(0x${hex4(u).toUpperCase()}, 0x${hex4(cu).toUpperCase()})`);
}
const space: string[] = [];
for (let u = 0; u < 0x10000; u++) {
  if (!/\s/.test(String.fromCharCode(u))) continue;
  let v = u;
  while (v + 1 < 0x10000 && /\s/.test(String.fromCharCode(v + 1))) v++;
  space.push(`(0x${hex4(u).toUpperCase()}, 0x${hex4(v).toUpperCase()})`);
  u = v;
}
const rows = (items: string[]) => {
  const out: string[] = [];
  for (let i = 0; i < items.length; i += 6) out.push(`    ${items.slice(i, i + 6).join(', ')},`);
  return out.join('\n');
};
writeFileSync(
  fileURLToPath(new URL('../../../crates/decay-core/src/js_case.rs', import.meta.url)),
  `//! Generated by \`apps/extension/scripts/policy-parity.ts\` from V8: do not edit.

/// JS \`Canonicalize\` for non-\`u\` case-insensitive patterns: every code unit whose single-unit
/// \`toUpperCase\` differs (non-ASCII never maps onto ASCII), as (unit, canonical), sorted.
#[rustfmt::skip]
pub(crate) static CANONICAL: &[(u16, u16)] = &[
${rows(canonical)}
];

/// The code units JS's \`\\s\` matches, as inclusive ranges.
#[rustfmt::skip]
pub(crate) static WHITE_SPACE: &[(u16, u16)] = &[
${rows(space)}
];
`,
);

const dir = new URL('../../../crates/decay-core/tests/fixtures/', import.meta.url);
writeFileSync(
  fileURLToPath(new URL('policy_parity.json', dir)),
  `${JSON.stringify({ pool: POOL, cases })}\n`,
);
writeFileSync(fileURLToPath(new URL('title_parity.json', dir)), `${JSON.stringify(titleCases)}\n`);
writeFileSync(fileURLToPath(new URL('fold_parity.json', dir)), `${JSON.stringify(foldCases)}\n`);
const counts: Record<string, number> = {};
for (const c of cases) counts[c.decision.action] = (counts[c.decision.action] ?? 0) + 1;
console.log(
  `wrote ${cases.length} policy cases`,
  counts,
  `${titleCases.length} title cases, ${foldCases.length} fold groups, ${canonical.length} canonical units`,
);
