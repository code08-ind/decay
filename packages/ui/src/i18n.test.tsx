import { render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import {
  availableLanguages,
  direction,
  formatList,
  formatMessage,
  getLocale,
  makeT,
  messageParams,
  onLocaleChange,
  parseMessage,
  pseudo,
  resolveLocale,
  setLocale,
  setSystemLocale,
} from './i18n';
import { richText } from './rich';

// Languages "with catalogs" for these tests (a package's translations add theirs).
makeT({}, { de: {}, pl: {}, pt: {}, he: {} });

afterEach(() => {
  setSystemLocale(null);
  setLocale('en');
});

describe('messages', () => {
  it('fills parameters, formatting numbers for the locale', () => {
    expect(formatMessage('Hello {name}', { name: 'Ada' })).toBe('Hello Ada');
    expect(formatMessage('{n} tabs', { n: 12345 })).toBe('12,345 tabs');
    setLocale('de');
    expect(formatMessage('{n} Tabs', { n: 12345 })).toBe('12.345 Tabs');
  });

  it('chooses plural branches by the locale’s rules, exact matches first', () => {
    const m = '{n, plural, =0 {Nothing} one {# tab} other {# tabs}} archived';
    expect(formatMessage(m, { n: 0 })).toBe('Nothing archived');
    expect(formatMessage(m, { n: 1 })).toBe('1 tab archived');
    expect(formatMessage(m, { n: 2500 })).toBe('2,500 tabs archived');
    setLocale('pl');
    const pl = '{n, plural, one {# karta} few {# karty} many {# kart} other {# karty}}';
    expect(formatMessage(pl, { n: 2 })).toBe('2 karty');
    expect(formatMessage(pl, { n: 5 })).toBe('5 kart');
  });

  it('selects, and nests', () => {
    const m = '{kind, select, tab {{n, plural, one {# tab} other {# tabs}}} other {# windows}}';
    expect(formatMessage(m, { kind: 'tab', n: 3 })).toBe('3 tabs');
    expect(formatMessage('{k, select, a {A {x}} other {O}}', { k: 'a', x: 'y' })).toBe('A y');
    expect(formatMessage('{k, select, a {A} other {O}}', { k: 'zzz' })).toBe('O');
  });

  it('keeps apostrophes and escapes doubled ones', () => {
    expect(formatMessage("Can't reach it")).toBe("Can't reach it");
    expect(formatMessage("It''s {x}", { x: 'ok' })).toBe("It's ok");
  });

  it('rejects malformed messages, and never breaks a screen over one', () => {
    for (const bad of [
      '{',
      'a}',
      '{n, plural, one {x}}',
      '{n, plural, one {x} other {y}',
      '{a b}',
    ]) {
      expect(() => parseMessage(bad), bad).toThrow();
    }
    expect(formatMessage('{n, plural, one {x}}', { n: 1 })).toBe('{n, plural, one {x}}');
  });

  it('takes `other` for a plural without a number, `#` as given', () => {
    const m = '{n, plural, =0 {none} one {# tab} other {# tabs}}';
    expect(formatMessage(m, {})).toBe(' tabs');
    expect(formatMessage(m, { n: 'lots' })).toBe('lots tabs');
    expect(formatMessage(m, { n: '' })).toBe(' tabs');
    expect(formatMessage(m, { n: '1' })).toBe('1 tab');
  });

  it('lists the parameters a message uses', () => {
    expect(messageParams('{a} {n, plural, one {# {b}} other {{c}}}')).toEqual(['a', 'b', 'c', 'n']);
  });

  it('formats lists in the locale’s words', () => {
    expect(formatList(['a', 'b', 'c'])).toBe('a, b, and c');
  });
});

describe('locales', () => {
  const en = { hello: 'Hello {name}', bye: 'Bye' } as const;
  const t = makeT(en, { fr: { hello: 'Bonjour {name}' }, 'fr-CA': { bye: 'Salut' } });

  it('looks up the full tag, then the language, then English', () => {
    setLocale('fr-CA');
    expect(t('bye')).toBe('Salut');
    expect(t('hello', { name: 'Ada' })).toBe('Bonjour Ada');
    setLocale('de');
    expect(t('hello', { name: 'Ada' })).toBe('Hello Ada');
  });

  it('follows the setting: system, a tag, or anything else as system', () => {
    expect(setLocale('system')).toBe(navigator.language);
    expect(setLocale('pt-br')).toBe('pt-BR');
    expect(setLocale(42)).toBe(navigator.language);
    expect(setLocale('not a tag!!')).toBe(navigator.language);
    setSystemLocale('de-AT'); // the OS language as the desktop app reads it
    expect(setLocale('system')).toBe('de-AT');
  });

  it('tells listeners, and sets lang and dir on the page', () => {
    let calls = 0;
    const off = onLocaleChange(() => calls++);
    setLocale('he');
    expect(calls).toBe(1);
    expect(document.documentElement.dir).toBe('rtl');
    expect(document.documentElement.lang).toBe('he');
    setLocale('he'); // no change: no call
    expect(calls).toBe(1);
    off();
    setLocale('en');
    expect(document.documentElement.dir).toBe('ltr');
    expect(calls).toBe(1);
  });

  it('is English (with the region) for a language without a catalog', () => {
    expect(availableLanguages()).toContain('en');
    expect(resolveLocale('ja-JP')).toBe('en-JP');
    expect(resolveLocale('ar')).toBe('en');
    expect(resolveLocale('en-GB')).toBe('en-GB');
    expect(resolveLocale('ar-XB')).toBe('ar-XB');
    setSystemLocale('ar-EG');
    expect(setLocale('system')).toBe('en-EG');
    expect(document.documentElement.dir).toBe('ltr');
    expect(formatMessage('{n}', { n: 1234 })).toBe('1,234');
  });

  it('knows which languages run right to left', () => {
    expect(direction('ar-EG')).toBe('rtl');
    expect(direction('fa')).toBe('rtl');
    expect(direction('en-GB')).toBe('ltr');
    expect(direction('ar-XB')).toBe('rtl');
  });

  it('has pseudo-locales that show what isn’t translated, and long text', () => {
    setLocale('en-XA');
    expect(getLocale()).toBe('en-XA');
    expect(t('bye')).toBe(pseudo('Bye'));
    expect(pseudo('Send feedback')).toBe('[Ŝéñð ƒééðбåçķ ~~~~]');
    setLocale('ar-XB');
    expect(document.documentElement.dir).toBe('rtl');
    expect(t('hello', { name: 'Ada' })).toMatch(/^\[Héļļö Åðå ~+\]$/);
  });
});

describe('rich text', () => {
  it('puts elements where the message says, in any order', () => {
    const t = makeT({ hint: 'You reopened {site} {n} times', fr: '{n} fois : {site}' });
    const { container } = render(
      <p>{richText((p) => t('hint', p), { site: <strong>a.com</strong>, n: 4 })}</p>,
    );
    expect(container.innerHTML).toBe('<p>You reopened <strong>a.com</strong> 4 times</p>');
    const fr = render(<p>{richText((p) => t('fr', p), { site: <b>b.org</b>, n: 2 })}</p>);
    expect(fr.container.innerHTML).toBe('<p>2 fois : <b>b.org</b></p>');
  });
});
