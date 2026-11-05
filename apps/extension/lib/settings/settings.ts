import { type DecayDB, db as defaultDb } from '../db/db';
import type { Mode, Thresholds } from '../db/types';

export type Preset = 'gentle' | 'balanced' | 'aggressive' | 'custom';

/** Active-hour thresholds per preset (Docs/PRD.md §5). */
export const PRESETS: Record<Exclude<Preset, 'custom'>, Thresholds> = {
  gentle: { stale: 24, hidden: 60, archive: 96 },
  balanced: { stale: 12, hidden: 36, archive: 60 },
  aggressive: { stale: 8, hidden: 24, archive: 40 },
};

/** Settings the extension uses in Phase 0. Keys mirror Docs/DB Design.md §3.9 (camelCase). */
export interface Settings {
  mode: Mode;
  shadowStartedAt: number | null;
  preset: Preset;
  thresholds: Thresholds;
  idleCutoffS: number;
  maxItemsPerSweep: number;
  retentionDays: number;
  holdUntil: number | null;
  theme: 'system' | 'light' | 'dark';
  notifications: { summary: boolean; sweep: boolean; health: boolean };
  locale: string;
}

export type SettingKey = keyof Settings;

export const DEFAULT_SETTINGS: Readonly<Settings> = Object.freeze({
  mode: 'shadow',
  shadowStartedAt: null, // set on first run
  preset: 'balanced',
  thresholds: PRESETS.balanced,
  idleCutoffS: 300,
  maxItemsPerSweep: 50,
  retentionDays: 90,
  holdUntil: null,
  theme: 'system',
  notifications: { summary: true, sweep: false, health: true },
  locale: 'system',
});

export function createSettingsStore(database: DecayDB = defaultDb) {
  async function get<K extends SettingKey>(key: K): Promise<Settings[K]> {
    const row = await database.settings.get(key);
    return row === undefined ? DEFAULT_SETTINGS[key] : (row.value as Settings[K]);
  }

  async function getAll(): Promise<Settings> {
    const rows = await database.settings.toArray();
    const stored = Object.fromEntries(rows.map((r) => [r.key, r.value]));
    return { ...DEFAULT_SETTINGS, ...stored } as Settings;
  }

  async function set<K extends SettingKey>(key: K, value: Settings[K]): Promise<void> {
    await database.settings.put({ key, value, updatedAt: Date.now() });
  }

  /** Writes several keys in one transaction. */
  async function update(patch: Partial<Settings>): Promise<void> {
    const now = Date.now();
    await database.settings.bulkPut(
      Object.entries(patch).map(([key, value]) => ({ key, value, updatedAt: now })),
    );
  }

  /** Picking a named preset also writes its thresholds. */
  async function setPreset(preset: Preset, custom?: Thresholds): Promise<void> {
    if (preset === 'custom') {
      if (!custom) throw new Error('Custom preset needs thresholds');
      assertValidThresholds(custom);
      await update({ preset, thresholds: custom });
    } else {
      await update({ preset, thresholds: PRESETS[preset] });
    }
  }

  /** Idempotent first-run setup: starts the shadow-mode clock once. */
  async function initFirstRun(now = Date.now()): Promise<void> {
    await database.transaction('rw', database.settings, async () => {
      if ((await database.settings.get('shadowStartedAt')) === undefined) {
        await database.settings.put({ key: 'shadowStartedAt', value: now, updatedAt: now });
      }
    });
  }

  return { get, getAll, set, update, setPreset, initFirstRun };
}

export function assertValidThresholds(t: Thresholds): void {
  const ok = [t.stale, t.hidden, t.archive].every((h) => Number.isFinite(h) && h > 0);
  if (!ok || !(t.stale <= t.hidden && t.hidden <= t.archive)) {
    throw new Error('Thresholds must be positive and ordered stale ≤ hidden ≤ archive');
  }
}

export const settings = createSettingsStore();
