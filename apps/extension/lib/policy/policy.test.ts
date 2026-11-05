import { describe, expect, it } from 'vitest';
import type { Rule } from '../db/types';
import {
  evaluate,
  isBlankPage,
  isExcluded,
  isRestorableUrl,
  type PolicyContext,
  type PolicyTarget,
  type TabSignals,
} from './policy';

const H = 3600;
const ctx = (over: Partial<PolicyContext> = {}): PolicyContext => ({
  now: 1_000_000,
  mode: 'automatic',
  holdUntil: null,
  thresholds: { stale: 12, hidden: 36, archive: 60 },
  rules: [],
  ...over,
});
const tab = (over: Partial<PolicyTarget> = {}): PolicyTarget => ({
  kind: 'tab',
  appId: 'com.google.Chrome',
  url: 'https://example.com/a',
  title: 'A',
  pinned: false,
  activeAgeS: 0,
  ...over,
});
const idle = { active: false, audible: false };
let n = 0;
const rule = (over: Partial<Rule>): Rule => ({
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

describe('evaluate', () => {
  it('applies the thresholds', () => {
    expect(evaluate(tab({ activeAgeS: 11 * H }), idle, ctx()).action).toBe('keep');
    expect(evaluate(tab({ activeAgeS: 12 * H }), idle, ctx()).action).toBe('stale');
    expect(evaluate(tab({ activeAgeS: 61.5 * H }), idle, ctx())).toEqual({
      action: 'archive',
      reason: { type: 'unused', activeHours: 61, threshold: 60 },
    });
  });

  it('holds and pause protect everything', () => {
    const old = tab({ activeAgeS: 100 * H });
    expect(evaluate(old, idle, ctx({ mode: 'paused' }))).toMatchObject({
      action: 'protect',
      reason: { by: 'hold' },
    });
    expect(evaluate(old, idle, ctx({ holdUntil: 2_000_000 })).action).toBe('protect');
    expect(evaluate(old, idle, ctx({ holdUntil: 500 })).action).toBe('archive'); // expired
  });

  it('applies hard protections in order', () => {
    const old = { activeAgeS: 100 * H };
    const by = (t: PolicyTarget, s: TabSignals = idle) => {
      const d = evaluate(t, s, ctx());
      return d.action === 'protect' ? d.reason.by : d.action;
    };
    expect(by(tab({ ...old, pinned: true }), { active: true, audible: true })).toBe('pinned');
    expect(by(tab(old), { active: true, audible: false })).toBe('activeTab');
    expect(by(tab(old), { active: false, audible: true })).toBe('audible');
    expect(by(tab(old), { active: false, audible: false, formInput: true })).toBe('formInput');
    expect(by(tab({ ...old, graceUntil: 2_000_000 }))).toBe('grace');
    expect(by(tab({ ...old, url: 'data:text/html,hi' }))).toBe('unrestorable');
    expect(by(tab({ ...old, url: undefined }))).toBe('unrestorable');
    expect(by(tab({ ...old, url: 'chrome://newtab/' }))).toBe('archive');
  });

  it('lets any protect rule win over custom thresholds', () => {
    const custom = rule({
      scope: 'urlGlob',
      match: 'https://example.com/*',
      action: 'customThreshold',
      thresholdHours: { stale: 1, hidden: 2, archive: 3 },
    });
    const protect = rule({ scope: 'domain', match: 'example.com' });
    const d = evaluate(tab({ activeAgeS: 100 * H }), idle, ctx({ rules: [custom, protect] }));
    expect(d).toEqual({
      action: 'protect',
      reason: { type: 'protected', by: 'rule', ruleId: protect.id },
    });
  });

  it('uses the most specific custom threshold', () => {
    const domainRule = rule({
      action: 'customThreshold',
      thresholdHours: { stale: 20, hidden: 30, archive: 40 },
    });
    const globRule = rule({
      scope: 'urlGlob',
      match: 'https://example.com/*',
      action: 'customThreshold',
      thresholdHours: { stale: 1, hidden: 2, archive: 3 },
    });
    const d = evaluate(tab({ activeAgeS: 4 * H }), idle, ctx({ rules: [domainRule, globRule] }));
    expect(d).toEqual({
      action: 'archive',
      reason: { type: 'customRule', ruleId: globRule.id, activeHours: 4 },
    });
  });

  it('ignores custom rules with invalid thresholds', () => {
    const legacy = rule({
      action: 'customThreshold',
      thresholdHours: { stale: 0, hidden: 0, archive: 0 },
    });
    expect(evaluate(tab({ activeAgeS: 1 }), idle, ctx({ rules: [legacy] })).action).toBe('keep');
  });

  it('ignores disabled, exclude and close-allowed rules', () => {
    const rules = [
      rule({ enabled: false }),
      rule({ action: 'exclude' }),
      rule({ scope: 'app', match: 'com.google.Chrome', action: 'closeAllowed' }),
    ];
    expect(evaluate(tab({ activeAgeS: 100 * H }), idle, ctx({ rules })).action).toBe('archive');
  });
});

describe('helpers', () => {
  it('knows which URLs reopen exactly', () => {
    for (const url of [
      'https://a.com',
      'http://x',
      'chrome://settings',
      'about:blank',
      'about:blank#x',
    ]) {
      expect(isRestorableUrl(url)).toBe(true);
    }
    for (const url of [
      'data:,x',
      'blob:https://a/1',
      'file:///a.pdf',
      'javascript:1',
      'about:srcdoc',
    ]) {
      expect(isRestorableUrl(url)).toBe(false);
    }
  });

  it('recognises blank pages', () => {
    expect(isBlankPage('chrome://newtab/')).toBe(true);
    expect(isBlankPage('about:blank')).toBe(true);
    expect(isBlankPage('chrome://settings/')).toBe(false);
    expect(isBlankPage(undefined)).toBe(false); // no URL = unrestorable, not blank
  });

  it('detects excluded targets', () => {
    const rules = [rule({ action: 'exclude', match: 'bank.com' })];
    expect(isExcluded({ appId: 'x', url: 'https://my.bank.com/', title: '' }, rules)).toBe(true);
    expect(isExcluded({ appId: 'x', url: 'https://other.com/', title: '' }, rules)).toBe(false);
  });
});
