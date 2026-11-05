import { type DecayDB, db as defaultDb } from '../db/db';
import type { Mode, Thresholds } from '../db/types';

export type Preset = 'gentle' | 'balanced' | 'aggressive' | 'custom';

type NamedPreset = Exclude<Preset, 'custom'>;

function deepFreeze<T>(obj: T): T {
  for (const value of Object.values(obj as object)) {
    if (value && typeof value === 'object') deepFreeze(value);
  }
  return Object.freeze(obj);
}

/** Active-hour thresholds per preset (Docs/PRD.md §5). */
export const PRESETS: Readonly<Record<NamedPreset, Readonly<Thresholds>>> = deepFreeze({
  gentle: { stale: 24, hidden: 60, archive: 96 },
  balanced: { stale: 12, hidden: 36, archive: 60 },
  aggressive: { stale: 8, hidden: 24, archive: 40 },
});

const PRESET_NAMES: readonly Preset[] = ['gentle', 'balanced', 'aggressive', 'custom'];
const MODES: readonly Mode[] = ['shadow', 'automatic', 'paused'];
const THEMES: readonly Settings['theme'][] = ['system', 'light', 'dark'];

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

export const DEFAULT_SETTINGS: Readonly<Settings> = deepFreeze({
  mode: 'shadow',
  shadowStartedAt: null, // set on first run
  preset: 'balanced',
  thresholds: { ...PRESETS.balanced },
  idleCutoffS: 300,
  maxItemsPerSweep: 50,
  retentionDays: 90,
  holdUntil: null,
  theme: 'system',
  notifications: { summary: true, sweep: false, health: true },
  locale: 'system',
});

/**
 * Validates a patch and keeps `preset` and `thresholds` consistent:
 * - a named preset writes its own thresholds (passing different ones is an error);
 * - `custom` requires thresholds;
 * - thresholds alone switch the preset to the matching named one, or to `custom`.
 * `undefined` values are dropped so they can never shadow a default.
 */
export function normalizePatch(patch: Partial<Settings>): Partial<Settings> {
  const out = Object.fromEntries(
    Object.entries(patch).filter(([, v]) => v !== undefined),
  ) as Partial<Settings>;

  if (out.mode !== undefined && !MODES.includes(out.mode)) {
    throw new Error(`Unknown mode: ${String(out.mode)}`);
  }
  if (out.theme !== undefined && !THEMES.includes(out.theme)) {
    throw new Error(`Unknown theme: ${String(out.theme)}`);
  }
  for (const key of ['idleCutoffS', 'maxItemsPerSweep', 'retentionDays'] as const) {
    const v = out[key];
    if (v !== undefined && !(Number.isInteger(v) && v > 0)) {
      throw new Error(`${key} must be a positive integer`);
    }
  }

  if (out.preset !== undefined) {
    if (!PRESET_NAMES.includes(out.preset)) {
      throw new Error(`Unknown preset: ${String(out.preset)}`);
    }
    if (out.preset === 'custom') {
      if (!out.thresholds) throw new Error('Custom preset needs thresholds');
      assertValidThresholds(out.thresholds);
    } else {
      const named = PRESETS[out.preset];
      if (out.thresholds && !sameThresholds(out.thresholds, named)) {
        throw new Error(`Thresholds do not match the ${out.preset} preset`);
      }
      out.thresholds = { ...named };
    }
  } else if (out.thresholds !== undefined) {
    assertValidThresholds(out.thresholds);
    out.preset = matchPreset(out.thresholds);
  }
  return out;
}

function sameThresholds(a: Thresholds, b: Thresholds): boolean {
  return a.stale === b.stale && a.hidden === b.hidden && a.archive === b.archive;
}

function matchPreset(t: Thresholds): Preset {
  const hit = (Object.keys(PRESETS) as NamedPreset[]).find((p) => sameThresholds(PRESETS[p], t));
  return hit ?? 'custom';
}

export function createSettingsStore(database: DecayDB = defaultDb) {
  // Reads return copies so callers can never mutate stored values or the defaults.
  async function get<K extends SettingKey>(key: K): Promise<Settings[K]> {
    const row = await database.settings.get(key);
    return structuredClone(row === undefined ? DEFAULT_SETTINGS[key] : (row.value as Settings[K]));
  }

  async function getAll(): Promise<Settings> {
    const rows = await database.settings.toArray();
    const stored = Object.fromEntries(rows.map((r) => [r.key, r.value]));
    return structuredClone({ ...DEFAULT_SETTINGS, ...stored }) as Settings;
  }

  /** Validates, then writes all keys in one transaction. */
  async function update(patch: Partial<Settings>): Promise<void> {
    const clean = normalizePatch(patch);
    const now = Date.now();
    await database.settings.bulkPut(
      Object.entries(clean).map(([key, value]) => ({ key, value, updatedAt: now })),
    );
  }

  async function set<K extends SettingKey>(key: K, value: Settings[K]): Promise<void> {
    await update({ [key]: value } as Partial<Settings>);
  }

  /** Picking a named preset also writes its thresholds; `custom` needs thresholds. */
  async function setPreset(preset: Preset, custom?: Thresholds): Promise<void> {
    await update({ preset, thresholds: preset === 'custom' ? custom : undefined });
  }

  /**
   * Idempotent first-run setup: starts the shadow-mode clock if it isn't set.
   * Safe to call on every service-worker start, so a missed `onInstalled` is recovered.
   */
  async function initFirstRun(now = Date.now()): Promise<void> {
    await database.transaction('rw', database.settings, async () => {
      const row = await database.settings.get('shadowStartedAt');
      if (row?.value == null) {
        await database.settings.put({ key: 'shadowStartedAt', value: now, updatedAt: now });
      }
    });
  }

  return { get, getAll, set, update, setPreset, initFirstRun };
}

export function assertValidThresholds(t: Thresholds): void {
  const values = [t?.stale, t?.hidden, t?.archive];
  const ok = values.every((h) => typeof h === 'number' && Number.isFinite(h) && h > 0);
  if (!ok || !(t.stale <= t.hidden && t.hidden <= t.archive)) {
    throw new Error('Thresholds must be positive and ordered stale ≤ hidden ≤ archive');
  }
}

export const settings = createSettingsStore();
