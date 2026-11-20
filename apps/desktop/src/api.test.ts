import { expect, it, describe as suite } from 'vitest';
import {
  appWithProfile,
  describe,
  type EngineStatus,
  errorMessage,
  exportSummary,
  importSummary,
  profileLabel,
  restoreChoices,
} from './api';

const base: EngineStatus = {
  mode: 'automatic',
  holdUntil: null,
  staleCount: 3,
  liveTabs: 12,
  browsers: [{ browser: 'chrome', profileId: 'p', tabs: 12 }],
  sweeping: false,
  undoableSweep: null,
  accessibility: true,
  inMeeting: false,
};

suite('describe', () => {
  it('matches the tray line', () => {
    expect(describe(base)).toBe('12 tabs open, 3 stale');
    expect(describe({ ...base, inMeeting: true })).toBe('In a meeting · sweeps wait');
    expect(describe({ ...base, mode: 'shadow', liveTabs: 1 })).toBe(
      '1 tab open, 3 stale · shadow mode',
    );
    expect(describe({ ...base, mode: 'paused' })).toBe('Paused');
    expect(describe({ ...base, browsers: [] })).toBe('No browser connected');
    expect(describe({ ...base, sweeping: true })).toBe('Sweeping…');
  });

  it('shows holds, and ignores one that ran out', () => {
    expect(describe({ ...base, holdUntil: 253_402_300_799_000 }, 0)).toBe('On hold');
    expect(describe({ ...base, holdUntil: 60_000 }, 0)).toMatch(/^On hold until /);
    expect(describe({ ...base, holdUntil: 1 }, 2)).toBe('12 tabs open, 3 stale');
  });
});

it('reads app errors', () => {
  expect(errorMessage({ code: 'CONFLICT', message: 'taken' })).toBe('taken');
  expect(errorMessage('plain')).toBe('plain');
});

suite('export / import summaries', () => {
  const none = {
    fileName: 'x.json',
    itemsImported: 0,
    itemsExisting: 0,
    itemsUpdated: 0,
    itemsLive: 0,
    itemsInvalid: 0,
    itemsExpired: 0,
    itemsExcluded: 0,
    itemsPurged: 0,
    rulesImported: 0,
    rulesExisting: 0,
    rulesInvalid: 0,
    rulesTurnedOff: 0,
    settingsImported: 0,
    linksCapped: false,
  };
  it('says what an import did', () => {
    expect(importSummary({ ...none, itemsImported: 1 })).toBe('Imported 1 item');
    expect(
      importSummary({
        ...none,
        itemsImported: 12,
        rulesImported: 2,
        settingsImported: 1,
        itemsExisting: 3,
        itemsUpdated: 1,
        itemsExcluded: 2,
        itemsInvalid: 1,
        itemsPurged: 1,
      }),
    ).toBe(
      "Imported 12 items, 2 rules, 1 setting (3 already here; 1 updated; 2 on never-recorded sites skipped; 1 unreadable or can't be reopened; 1 stored item deleted by its rules)",
    );
  });
  it('says why items were skipped', () => {
    expect(
      importSummary({
        ...none,
        itemsExpired: 4,
        itemsLive: 1,
        rulesInvalid: 2,
        rulesTurnedOff: 1,
        linksCapped: true,
      }),
    ).toBe(
      'Imported 0 items (4 older than you keep items, skipped; 1 open tab left to the browser; 2 rules not accepted; 1 built-in protection turned off; only the first 100,000 links read)',
    );
  });
  it('names the exported file on either platform', () => {
    expect(exportSummary({ path: '/Users/me/Decay archive.json', count: 3 })).toBe(
      'Exported 3 items to Decay archive.json',
    );
    expect(exportSummary({ path: 'C:\\Users\\me\\a.csv', count: 1 })).toBe(
      'Exported 1 item to a.csv',
    );
  });
});

suite('profile labels', () => {
  it('names the profile next to its app, or just the app', () => {
    expect(appWithProfile('Google Chrome', null)).toBe('Google Chrome');
    expect(appWithProfile('Google Chrome', { id: 'a', name: 'Work', ordinal: 2 })).toBe(
      'Google Chrome · Work',
    );
    expect(appWithProfile('Arc', { id: 'a', name: null, ordinal: 2 })).toBe('Arc · Profile 2');
    const p = {
      appId: 'c',
      appName: 'Brave',
      profileId: 'x',
      name: null,
      ordinal: 1,
      hasSiblings: false,
      connected: true,
      tabs: 0,
      firstSeenAt: 0,
      lastSeenAt: 0,
    };
    expect(profileLabel(p)).toBe('Brave');
    expect(profileLabel({ ...p, hasSiblings: true })).toBe('Brave · Profile 1');
    expect(profileLabel({ ...p, name: 'Home' })).toBe('Brave · Home');
  });
});

suite('restoreChoices', () => {
  const alt = { appId: 'arc', profileId: 'p', label: 'Arc' };
  it('reads a refusal that offers other profiles, and nothing else', () => {
    const e = {
      code: 'ADAPTER_UNAVAILABLE',
      message: 'm',
      details: { itemIds: ['a'], home: null, alternatives: [alt] },
    };
    expect(restoreChoices(e)).toEqual({ itemIds: ['a'], alternatives: [alt] });
    expect(restoreChoices({ ...e, code: 'NOT_FOUND' })).toBeNull();
    expect(restoreChoices({ ...e, details: { itemIds: ['a'], alternatives: [] } })).toBeNull();
    expect(restoreChoices({ ...e, details: { itemIds: [], alternatives: [alt] } })).toBeNull();
    expect(restoreChoices({ code: 'ADAPTER_UNAVAILABLE', message: 'm' })).toBeNull();
    expect(restoreChoices('boom')).toBeNull();
    const many = {
      ...e,
      details: { itemIds: ['a'], alternatives: [alt, alt, alt, alt, { bad: 1 }] },
    };
    expect(restoreChoices(many)?.alternatives).toHaveLength(3);
  });
});
