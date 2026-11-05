import { describe, expect, it } from 'vitest';
import type { Rule } from '../db/types';
import { domainMatches, globMatches, hostOf, ruleMatches, titleRegexProblem } from './match';

const rule = (scope: Rule['scope'], match: string, enabled = true): Rule => ({
  id: 'r',
  scope,
  match,
  action: 'protect',
  enabled,
  source: 'user',
  createdAt: 0,
  hitCount: 0,
});
const target = (url?: string, title = 'Title') => ({ appId: 'com.google.Chrome', url, title });

describe('match', () => {
  it('extracts hosts from web URLs only', () => {
    expect(hostOf('https://Docs.GitHub.com./x')).toBe('docs.github.com');
    expect(hostOf('chrome://settings')).toBeUndefined();
    expect(hostOf('not a url')).toBeUndefined();
  });

  it('matches a domain and its subdomains, not look-alikes', () => {
    expect(domainMatches('github.com', 'github.com')).toBe(true);
    expect(domainMatches('gist.github.com', 'github.com')).toBe(true);
    expect(domainMatches('gist.github.com', '*.github.com')).toBe(true);
    expect(domainMatches('notgithub.com', 'github.com')).toBe(false);
    expect(domainMatches('github.com.evil.io', 'github.com')).toBe(false);
    expect(domainMatches(undefined, 'github.com')).toBe(false);
  });

  it('matches globs with only * as a wildcard', () => {
    expect(globMatches('https://*.notion.so/*', 'https://acme.notion.so/page')).toBe(true);
    expect(globMatches('https://*.notion.so/*', 'https://notion.so.evil/x')).toBe(false);
    expect(globMatches('https://a.com/?q=1', 'https://a.com/?q=1')).toBe(true); // ? is literal
    expect(globMatches('https://a.com/?q=1', 'https://a.com/xq=1')).toBe(false);
    expect(globMatches('https://a.com/(x)+', 'https://A.com/(x)+')).toBe(true);
    expect(globMatches('*', '')).toBe(true);
    expect(globMatches('a*b*c', 'axxbyyc')).toBe(true);
    expect(globMatches('a*b*c', 'axxbyy')).toBe(false);
  });

  it('globs ignore userinfo tricks', () => {
    const r = rule('urlGlob', 'https://github.com*');
    expect(ruleMatches(r, target('https://github.com@evil.com/'))).toBe(false);
    expect(ruleMatches(r, target('https://github.com/x'))).toBe(true);
  });

  it('globs stay fast on long non-matching URLs', () => {
    const r = rule('urlGlob', 'https://*a*b*c*d*e*f*g*h*zzz');
    const url = `https://example.com/${'abcdefgh'.repeat(300)}`;
    const start = performance.now();
    expect(ruleMatches(r, target(url))).toBe(false);
    expect(performance.now() - start).toBeLessThan(200);
  });

  it('refuses title patterns that can backtrack catastrophically', () => {
    for (const bad of [
      '(a+)+$',
      '(a|aa)*',
      '(\\w+\\s?)*x',
      '(.*)*',
      '(a)\\1',
      '(?=a)a',
      '.*.*.*x',
    ]) {
      expect(titleRegexProblem(bad), bad).not.toBeNull();
    }
    for (const ok of ['^Inbox', 'jira|confluence', '\\(\\d+\\) unread', '.*bank.*', '[a-z]{2,5}']) {
      expect(titleRegexProblem(ok), ok).toBeNull();
    }
    // An unsafe pattern written straight into the DB never runs.
    const start = performance.now();
    expect(ruleMatches(rule('titleRegex', '(a+)+$'), target('x', `${'a'.repeat(40)}!`))).toBe(
      false,
    );
    expect(performance.now() - start).toBeLessThan(50);
  });

  it('matches IP addresses exactly', () => {
    expect(domainMatches('192.168.1.1', '1.1')).toBe(false);
    expect(domainMatches('10.0.0.1', '0.1')).toBe(false);
    expect(domainMatches('10.0.0.1', '10.0.0.1')).toBe(true);
  });

  it('matches each scope and ignores disabled rules', () => {
    const t = target('https://app.figma.com/file/1', 'Design — Figma');
    expect(ruleMatches(rule('global', '*'), t)).toBe(true);
    expect(ruleMatches(rule('app', 'com.google.Chrome'), t)).toBe(true);
    expect(ruleMatches(rule('app', 'com.microsoft.Edge'), t)).toBe(false);
    expect(ruleMatches(rule('domain', 'figma.com'), t)).toBe(true);
    expect(ruleMatches(rule('urlGlob', 'https://app.figma.com/file/*'), t)).toBe(true);
    expect(ruleMatches(rule('titleRegex', 'figma$'), t)).toBe(true);
    expect(ruleMatches(rule('titleRegex', '(unclosed'), t)).toBe(false);
    expect(ruleMatches(rule('domain', 'figma.com', false), t)).toBe(false);
  });
});
