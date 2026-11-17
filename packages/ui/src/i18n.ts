// Localization (P2.7, PRD F31): every string people see comes from a catalog, English
// at launch. Each package keeps its own typed English catalog and makes its `t` with
// `makeT`; they all share one current locale (the `locale` setting, `system` = the
// browser's language), which also sets number / date formats, plural rules and the
// writing direction.
//
// Message syntax (a small ICU subset):
//   {name}                                   a parameter (numbers formatted for the locale)
//   {n, plural, =0 {none} one {# tab} other {# tabs}}   `#` = n, formatted
//   {kind, select, tab {tab} window {window} other {item}}
// Branches may hold parameters and further plurals / selects.
//
// Pseudo-locales find what isn't translated and what breaks: `en-XA` accents and
// lengthens every message ([Ŝéñð ƒééðбåçķ ~~~]); `ar-XB` does too, right to left.

export type Catalog = Record<string, string>;
export type Params = Record<string, string | number | null | undefined>;

const RTL_LANGUAGES = new Set(['ar', 'fa', 'he', 'iw', 'ps', 'ur', 'yi', 'dv', 'ckb', 'sd']);
const PSEUDO = new Set(['en-XA', 'ar-XB']);

/** Languages with catalogs: English, and those of every package's translations (`makeT`). */
const languages = new Set(['en']);

/** The languages Decay has text in (English at launch). */
export function availableLanguages(): string[] {
  return [...languages];
}

let current = 'en';
let system: string | null = null;
const listeners = new Set<() => void>();

/**
 * The system's language as the app reads it (the desktop app's tray does; a web view may
 * report its own). Until set, the browser's.
 */
export function setSystemLocale(tag: string | null): void {
  system = tag;
}

/** The system's language (the browser's, unless set), or English. */
export function systemLocale(): string {
  if (system) return system;
  try {
    return (typeof navigator !== 'undefined' && navigator.language) || 'en';
  } catch {
    return 'en';
  }
}

/** A BCP 47 tag that `Intl` accepts, or `null`. */
function canonical(tag: string): string | null {
  if (PSEUDO.has(tag)) return tag;
  try {
    return Intl.getCanonicalLocales(tag)[0] ?? null;
  } catch {
    return null;
  }
}

/**
 * The locale a tag resolves to: itself when its language has a catalog (or it's a
 * pseudo-locale), else English with the tag's region (`de-AT` → `en-AT`): English text
 * isn't written right to left or counted with another language's plural rules. The
 * desktop app's Rust side resolves the same way.
 */
export function resolveLocale(tag: string): string | null {
  const c = canonical(tag);
  if (!c || PSEUDO.has(c)) return c;
  const lang = c.split('-')[0]?.toLowerCase() ?? '';
  if (languages.has(lang)) return c;
  let region: string | undefined;
  try {
    region = new Intl.Locale(c).region;
  } catch {
    region = undefined;
  }
  return (region && canonical(`en-${region}`)) || 'en';
}

/** Sets the locale from the `locale` setting (`system`, a tag, or anything else → system). */
export function setLocale(setting: unknown): string {
  const wanted = typeof setting === 'string' && setting !== 'system' ? setting : systemLocale();
  const next = resolveLocale(wanted) ?? resolveLocale(systemLocale()) ?? 'en';
  if (next !== current) {
    current = next;
    applyDirection();
    for (const f of listeners) f();
  } else {
    applyDirection();
  }
  return current;
}

export function getLocale(): string {
  return current;
}

/** Called when the locale changes (re-render). Returns the unsubscribe. */
export function onLocaleChange(f: () => void): () => void {
  listeners.add(f);
  return () => listeners.delete(f);
}

/** The locale `Intl` formats with (a pseudo-locale formats as its base). */
function intlLocale(locale = current): string {
  if (locale === 'en-XA') return 'en';
  if (locale === 'ar-XB') return 'ar';
  return locale;
}

/** `rtl` for right-to-left languages. */
export function direction(locale = current): 'ltr' | 'rtl' {
  if (locale === 'ar-XB') return 'rtl';
  const lang = locale.split('-')[0]?.toLowerCase() ?? '';
  return RTL_LANGUAGES.has(lang) ? 'rtl' : 'ltr';
}

/** Puts `lang` and `dir` on the page, so layouts and text flip. */
function applyDirection() {
  if (typeof document === 'undefined') return;
  document.documentElement.lang = current;
  document.documentElement.dir = direction();
}

/** ArrowLeft / ArrowRight as "previous" / "next" in reading order (swapped right to left). */
export function logicalArrow(key: string): string {
  if (direction() !== 'rtl') return key;
  return key === 'ArrowLeft' ? 'ArrowRight' : key === 'ArrowRight' ? 'ArrowLeft' : key;
}

export function formatNumber(n: number, options?: Intl.NumberFormatOptions): string {
  try {
    return new Intl.NumberFormat(intlLocale(), options).format(n);
  } catch {
    return String(n);
  }
}

export function formatDate(at: number | Date, options?: Intl.DateTimeFormatOptions): string {
  try {
    return new Intl.DateTimeFormat(intlLocale(), options).format(at);
  } catch {
    return new Date(at).toISOString();
  }
}

/** A list in the locale's words: "a, b and c". */
export function formatList(items: string[], type: 'conjunction' | 'disjunction' = 'conjunction') {
  try {
    return new Intl.ListFormat(intlLocale(), { style: 'long', type }).format(items);
  } catch {
    return items.join(', ');
  }
}

const pluralRules = new Map<string, Intl.PluralRules>();
function pluralCategory(n: number): string {
  const loc = intlLocale();
  let rules = pluralRules.get(loc);
  if (!rules) {
    try {
      rules = new Intl.PluralRules(loc);
    } catch {
      rules = new Intl.PluralRules('en');
    }
    pluralRules.set(loc, rules);
  }
  return rules.select(n);
}

// --- message formatting -------------------------------------------------------------

type Node =
  | string
  | { arg: string }
  | { arg: string; kind: 'plural' | 'select'; branches: Record<string, Node[]> }
  | { hash: true };

/** Parses a message; throws on malformed syntax (the catalog tests catch it). */
export function parseMessage(message: string): Node[] {
  let i = 0;
  const parse = (inBranch: boolean, inPlural: boolean): Node[] => {
    const out: Node[] = [];
    let text = '';
    const flush = () => {
      if (text) out.push(text);
      text = '';
    };
    while (i < message.length) {
      const c = message[i];
      if (c === "'" && message[i + 1] === "'") {
        text += "'";
        i += 2;
      } else if (c === '}') {
        if (!inBranch) throw new Error(`unexpected } at ${i} in "${message}"`);
        flush();
        return out;
      } else if (c === '#' && inPlural) {
        flush();
        out.push({ hash: true });
        i++;
      } else if (c === '{') {
        flush();
        i++;
        const close = message.indexOf('}', i);
        const comma = message.indexOf(',', i);
        if (close < 0) throw new Error(`unclosed { in "${message}"`);
        if (comma < 0 || comma > close) {
          const name = message.slice(i, close).trim();
          if (!/^\w+$/.test(name)) throw new Error(`bad parameter "${name}" in "${message}"`);
          out.push({ arg: name });
          i = close + 1;
        } else {
          const name = message.slice(i, comma).trim();
          const rest = message.slice(comma + 1);
          const m = /^\s*(plural|select)\s*,/.exec(rest);
          if (!m || !/^\w+$/.test(name)) throw new Error(`bad argument in "${message}"`);
          const kind = m[1] as 'plural' | 'select';
          i = comma + 1 + m[0].length;
          const branches: Record<string, Node[]> = {};
          for (;;) {
            while (/\s/.test(message[i] ?? '')) i++;
            if (message[i] === '}') {
              i++;
              break;
            }
            const key = /^(=\d+|\w+)\s*\{/.exec(message.slice(i));
            if (!key) throw new Error(`bad branch in "${message}"`);
            i += key[0].length;
            branches[key[1] as string] = parse(true, kind === 'plural' || inPlural);
            if (message[i] !== '}') throw new Error(`unclosed branch in "${message}"`);
            i++;
          }
          if (!branches.other) throw new Error(`"${name}" needs an other branch in "${message}"`);
          out.push({ arg: name, kind, branches });
        }
      } else {
        text += c;
        i++;
      }
    }
    if (inBranch) throw new Error(`unclosed branch in "${message}"`);
    flush();
    return out;
  };
  return parse(false, false);
}

/** The parameter names a message uses. */
export function messageParams(message: string): string[] {
  const names = new Set<string>();
  const walk = (nodes: Node[]) => {
    for (const n of nodes) {
      if (typeof n === 'string' || 'hash' in n) continue;
      names.add(n.arg);
      if ('branches' in n) for (const b of Object.values(n.branches)) walk(b);
    }
  };
  walk(parseMessage(message));
  return [...names].sort();
}

const parsed = new Map<string, Node[]>();

function render(nodes: Node[], params: Params, hash?: string): string {
  let out = '';
  for (const n of nodes) {
    if (typeof n === 'string') out += n;
    else if ('hash' in n) out += hash ?? '#';
    else if ('branches' in n) {
      const v = params[n.arg];
      if (n.kind === 'plural') {
        // No number (missing, or text that isn't one): `other`, with `#` as given. The
        // Rust side does the same.
        const num =
          typeof v === 'number' ? v : typeof v === 'string' && v.trim() !== '' ? Number(v) : NaN;
        const known = Number.isFinite(num);
        const branch =
          (known && (n.branches[`=${num}`] ?? n.branches[pluralCategory(num)])) || n.branches.other;
        out += render(branch ?? [], params, known ? formatNumber(num) : String(v ?? ''));
      } else {
        out += render(n.branches[String(v)] ?? n.branches.other ?? [], params, hash);
      }
    } else {
      const v = params[n.arg];
      out += typeof v === 'number' ? formatNumber(v) : (v ?? '');
    }
  }
  return out;
}

/** Formats a message with `params`. */
export function formatMessage(message: string, params: Params = {}): string {
  let nodes = parsed.get(message);
  if (!nodes) {
    try {
      nodes = parseMessage(message);
    } catch {
      return message; // never break a screen over a bad translation
    }
    parsed.set(message, nodes);
  }
  return render(nodes, params);
}

const ACCENTS: Record<string, string> = {
  a: 'å',
  b: 'б',
  c: 'ç',
  d: 'ð',
  e: 'é',
  f: 'ƒ',
  g: 'ĝ',
  h: 'ĥ',
  i: 'î',
  j: 'ĵ',
  k: 'ķ',
  l: 'ļ',
  m: 'ɱ',
  n: 'ñ',
  o: 'ö',
  p: 'þ',
  r: 'ŕ',
  s: 'š',
  t: 'ţ',
  u: 'û',
  w: 'ŵ',
  y: 'ý',
  z: 'ž',
  A: 'Å',
  C: 'Ç',
  D: 'Ð',
  E: 'É',
  I: 'Î',
  N: 'Ñ',
  O: 'Ö',
  S: 'Ŝ',
  U: 'Û',
};

/** The pseudo-locale form: accented, ~30 % longer, bracketed. */
export function pseudo(text: string): string {
  const accented = [...text].map((ch) => ACCENTS[ch] ?? ch).join('');
  const pad = '~'.repeat(Math.max(1, Math.round(text.length * 0.3)));
  return `[${accented} ${pad}]`;
}

/**
 * A package's `t`: `t('key', params)` with the keys of its English catalog. Translations
 * (`{ fr: { key: '…' } }`) are looked up by full tag, then language; missing keys fall
 * back to English.
 */
export function makeT<K extends string>(
  en: Readonly<Record<K, string>>,
  translations: Record<string, Partial<Record<K, string>>> = {},
) {
  for (const tag of Object.keys(translations)) {
    const lang = tag.split('-')[0]?.toLowerCase();
    if (lang) languages.add(lang);
  }
  return function t(key: K, params?: Params): string {
    const loc = current;
    const lang = loc.split('-')[0] ?? '';
    const message = translations[loc]?.[key] ?? translations[lang]?.[key] ?? en[key] ?? key;
    const text = formatMessage(message, params);
    return PSEUDO.has(loc) ? pseudo(text) : text;
  };
}
