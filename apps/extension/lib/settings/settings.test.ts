import { pseudo, setLocale } from '@decay/ui';
import { afterEach, describe, expect, it } from 'vitest';
import { DecayDB } from '../db/db';
import { createSettingsStore, DEFAULT_SETTINGS, normalizePatch, PRESETS } from './settings';

let n = 0;
const open: DecayDB[] = [];
function freshStore() {
  const d = new DecayDB(`decay-settings-test-${n++}`);
  open.push(d);
  return createSettingsStore(d);
}
afterEach(async () => {
  await Promise.all(open.splice(0).map((d) => d.delete()));
});

describe('settings', () => {
  it('returns defaults when nothing is stored', async () => {
    const s = freshStore();
    expect(await s.getAll()).toEqual(DEFAULT_SETTINGS);
    expect(await s.get('mode')).toBe('shadow');
    expect(await s.get('thresholds')).toEqual({ stale: 12, hidden: 36, archive: 60 });
  });

  it('persists a value', async () => {
    const s = freshStore();
    await s.set('mode', 'automatic');
    expect(await s.get('mode')).toBe('automatic');
  });

  it('writes preset thresholds together with the preset', async () => {
    const s = freshStore();
    await s.setPreset('gentle');
    const all = await s.getAll();
    expect(all.preset).toBe('gentle');
    expect(all.thresholds).toEqual(PRESETS.gentle);
  });

  it('validates custom thresholds', async () => {
    const s = freshStore();
    await expect(s.setPreset('custom', { stale: 40, hidden: 30, archive: 60 })).rejects.toThrow();
    await s.setPreset('custom', { stale: 6, hidden: 20, archive: 30 });
    expect(await s.get('thresholds')).toEqual({ stale: 6, hidden: 20, archive: 30 });
  });

  it('starts the shadow clock only once', async () => {
    const s = freshStore();
    await s.initFirstRun(1000);
    await s.initFirstRun(2000);
    expect(await s.get('shadowStartedAt')).toBe(1000);
  });

  it('hands out copies, never the shared defaults', async () => {
    const s = freshStore();
    const all = await s.getAll();
    all.thresholds.stale = 999;
    all.notifications.summary = false;
    (await s.get('thresholds')).archive = 1;
    expect(PRESETS.balanced.stale).toBe(12);
    expect(DEFAULT_SETTINGS.notifications.summary).toBe(true);
    expect(await s.get('thresholds')).toEqual({ stale: 12, hidden: 36, archive: 60 });
    await s.setPreset('balanced');
    expect(await s.get('thresholds')).toEqual({ stale: 12, hidden: 36, archive: 60 });
  });

  it('rejects unknown presets, modes and bad numbers', async () => {
    const s = freshStore();
    // @ts-expect-error runtime guard for untyped callers
    await expect(s.setPreset('turbo')).rejects.toThrow();
    // @ts-expect-error runtime guard for untyped callers
    await expect(s.set('mode', 'yolo')).rejects.toThrow();
    await expect(s.set('retentionDays', 0)).rejects.toThrow();
    await expect(s.update({ preset: 'custom' })).rejects.toThrow();
    await expect(
      s.set('thresholds', { stale: -5, hidden: 0, archive: Number.NaN }),
    ).rejects.toThrow();
    await expect(
      s.update({ preset: 'balanced', thresholds: { stale: 1, hidden: 2, archive: 3 } }),
    ).rejects.toThrow();
    expect(await s.getAll()).toEqual(DEFAULT_SETTINGS);
  });

  it('keeps preset in sync when thresholds are written alone', async () => {
    const s = freshStore();
    await s.set('thresholds', { stale: 6, hidden: 20, archive: 30 });
    expect(await s.get('preset')).toBe('custom');
    await s.set('thresholds', { ...PRESETS.aggressive });
    expect(await s.get('preset')).toBe('aggressive');
  });

  it('ignores undefined values instead of hiding defaults', async () => {
    const s = freshStore();
    await s.update({ mode: undefined });
    expect(await s.get('mode')).toBe('shadow');
    expect((await s.getAll()).mode).toBe('shadow');
  });

  it('starts the shadow clock if it was stored as null', async () => {
    const s = freshStore();
    await s.set('shadowStartedAt', null);
    await s.initFirstRun(5000);
    expect(await s.get('shadowStartedAt')).toBe(5000);
  });

  it('merges stored values over defaults', async () => {
    const s = freshStore();
    await s.update({ theme: 'dark', retentionDays: 30 });
    expect(await s.getAll()).toEqual({ ...DEFAULT_SETTINGS, theme: 'dark', retentionDays: 30 });
  });

  it('falls back to preset thresholds when stored ones are invalid', async () => {
    const s = freshStore();
    for (const bad of [
      { stale: 12, hidden: 12, archive: 60 },
      { stale: 0.25, hidden: 0.5, archive: 2000 },
      { stale: 'x' },
    ]) {
      await open[open.length - 1]?.settings.put({ key: 'thresholds', value: bad, updatedAt: 0 });
      await open[open.length - 1]?.settings.put({ key: 'preset', value: 'gentle', updatedAt: 0 });
      expect(await s.get('thresholds')).toEqual(PRESETS.gentle);
      expect((await s.getAll()).thresholds).toEqual(PRESETS.gentle);
    }
  });

  it('enforces the 1–720 h range and strict order', async () => {
    const s = freshStore();
    await s.setPreset('custom', { stale: 1, hidden: 2, archive: 720 });
    for (const bad of [
      { stale: 0.5, hidden: 2, archive: 3 },
      { stale: 1, hidden: 2, archive: 721 },
      { stale: 5, hidden: 5, archive: 9 },
    ]) {
      await expect(s.setPreset('custom', bad)).rejects.toThrow();
    }
  });
});

describe('settings errors', () => {
  afterEach(() => setLocale('en'));

  it('are in the user’s language', () => {
    expect(() => normalizePatch({ mode: 'nope' as never })).toThrow('Unknown mode: nope');
    expect(() => normalizePatch({ thresholds: { stale: 9, hidden: 4, archive: 8 } })).toThrow(
      'Thresholds must be 1–720 h and ordered stale < hidden < archive',
    );
    setLocale('en-XA');
    expect(() => normalizePatch({ preset: 'custom' })).toThrow(
      pseudo('Custom preset needs thresholds'),
    );
  });
});
