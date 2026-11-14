import { describe, expect, it } from 'vitest';
import {
  acceleratorFrom,
  actionChip,
  BALANCED,
  factorOf,
  intentOf,
  PRESET_HOURS,
  paceOf,
  ruleInput,
  ruleSentence,
  scaled,
  sliderFactor,
  tabFor,
  thresholdsProblem,
  workTime,
} from './model';

const key = (
  k: string,
  code: string,
  mods: Partial<Record<'alt' | 'ctrl' | 'meta' | 'shift', boolean>> = {},
) => ({
  key: k,
  code,
  altKey: !!mods.alt,
  ctrlKey: !!mods.ctrl,
  metaKey: !!mods.meta,
  shiftKey: !!mods.shift,
});

describe('settings model', () => {
  it('maps routes to tabs', () => {
    expect(tabFor('/aging')).toBe('aging');
    expect(tabFor('/protection?x')).toBe('protection');
    expect(tabFor('/license?activate')).toBe('general');
    expect(tabFor('/')).toBe('general');
  });

  it('checks thresholds like the core', () => {
    expect(thresholdsProblem(BALANCED)).toBeNull();
    expect(thresholdsProblem({ stale: 0, hidden: 2, archive: 3 })).toMatch(/1 to 720/);
    expect(thresholdsProblem({ stale: 5, hidden: 5, archive: 9 })).toMatch(/before/);
    expect(thresholdsProblem({ stale: 1, hidden: 2, archive: 721 })).toMatch(/1 to 720/);
  });

  it('names the pace of a set of thresholds', () => {
    expect(paceOf(PRESET_HOURS.gentle)).toBe('gentle');
    expect(paceOf({ stale: 12, hidden: 36, archive: 61 })).toBe('custom');
    expect(paceOf(undefined)).toBe('balanced');
  });

  it('says hours as work time', () => {
    expect(workTime(4)).toBe('≈ 4 h of work');
    expect(workTime(12)).toBe('≈ 1.5 work days');
    expect(workTime(60)).toBe('≈ 1.5 work weeks');
  });

  it('scales thresholds and keeps them valid', () => {
    expect(scaled(BALANCED, 2)).toEqual({ stale: 24, hidden: 72, archive: 120 });
    expect(scaled(BALANCED, 0.25)).toEqual({ stale: 3, hidden: 9, archive: 15 });
    for (const f of [0.01, 0.2, 1, 3, 5, 50]) {
      expect(thresholdsProblem(scaled(BALANCED, f))).toBeNull();
    }
    expect(factorOf({ thresholdHours: { stale: 24, hidden: 72, archive: 120 } }, BALANCED)).toBe(2);
    expect(factorOf({}, BALANCED)).toBe(1);
  });

  it('describes rules and builds their input', () => {
    const longer = ruleInput('domain', 'a.com', 'longer', 2, BALANCED);
    expect(longer).toEqual({
      scope: 'domain',
      match: 'a.com',
      action: 'customThreshold',
      thresholdHours: { stale: 24, hidden: 72, archive: 120 },
      enabled: true,
    });
    const sooner = ruleInput('domain', 'a.com', 'sooner', 4, BALANCED);
    expect(sooner.thresholdHours?.archive).toBe(15);
    expect(intentOf(longer, BALANCED)).toBe('longer');
    expect(intentOf(sooner, BALANCED)).toBe('sooner');
    expect(actionChip(sooner, BALANCED)).toBe('Archive sooner');
    expect(actionChip({ action: 'protect' }, BALANCED)).toBe('Never archive');
    expect(ruleSentence({ ...longer, scope: 'domain' })).toBe('Archived after 120 active h');
    expect(ruleInput('urlGlob', 'x/*', 'exclude', 1, BALANCED, false)).toEqual({
      scope: 'urlGlob',
      match: 'x/*',
      action: 'exclude',
      enabled: false,
    });
    expect(intentOf({ action: 'exclude' }, BALANCED)).toBe('exclude');
    expect(ruleSentence({ action: 'protect', scope: 'domain' })).toBe('Never archived');
  });

  it('records global shortcuts from key presses', () => {
    expect(acceleratorFrom(key('∂', 'KeyD', { alt: true, meta: true }))).toBe('Alt+Super+D');
    expect(acceleratorFrom(key('d', 'KeyD', { ctrl: true, alt: true }))).toBe('Ctrl+Alt+D');
    expect(acceleratorFrom(key(' ', 'Space', { ctrl: true, shift: true }))).toBe(
      'Ctrl+Shift+Space',
    );
    expect(acceleratorFrom(key('F5', 'F5', { meta: true }))).toBe('Super+F5');
    expect(acceleratorFrom(key('1', 'Digit1', { ctrl: true }))).toBe('Ctrl+1');
    // Modifiers alone, no modifier, or Shift alone: not a global shortcut yet.
    expect(acceleratorFrom(key('Alt', 'AltLeft', { alt: true }))).toBeNull();
    expect(acceleratorFrom(key('d', 'KeyD'))).toBeNull();
    expect(acceleratorFrom(key('D', 'KeyD', { shift: true }))).toBeNull();
    expect(acceleratorFrom(key('Dead', 'Unidentified', { alt: true }))).toBeNull();
  });
});

describe('sliderFactor', () => {
  const g = { stale: 12, hidden: 36, archive: 60 };
  it('stays on the slider, whatever the stored hours', () => {
    expect(sliderFactor({ thresholdHours: { stale: 1, hidden: 2, archive: 120 } }, g)).toBe(2);
    expect(sliderFactor({ thresholdHours: { stale: 1, hidden: 2, archive: 30 } }, g)).toBe(2);
    expect(sliderFactor({ thresholdHours: { stale: 1, hidden: 2, archive: 3 } }, g)).toBe(5);
    expect(sliderFactor({ thresholdHours: { stale: 1, hidden: 2, archive: 60 } }, g)).toBe(1.25);
    expect(sliderFactor({ thresholdHours: { stale: 1, hidden: 2, archive: 720 } }, g)).toBe(5);
  });
});
