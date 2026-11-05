import { afterEach, describe, expect, it } from 'vitest';
import { DecayDB } from '../db/db';
import { createSettingsStore, DEFAULT_SETTINGS, PRESETS } from './settings';

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
});
