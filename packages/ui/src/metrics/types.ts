/** On-device success metrics shared by the extension and the desktop app (F34). */

/** Decay's mode (the same values in the extension and the app). */
export type Mode = 'shadow' | 'automatic' | 'paused';

/** One local day of metrics (the extension's IndexedDB row; the app sends the same shape). */
export interface MetricsDay {
  day: string; // YYYY-MM-DD, local
  activeSeconds: number;
  openTabsP50: number;
  openWindowsP50: number;
  archivedCount: number;
  restored24hCount: number;
  undoCount: number;
  searchCount: number;
  searchToRestoreMsP50: number;
  autoModeOn: 0 | 1;
  /** Phase 0: items a shadow sweep flagged as "would archive" this day. */
  shadowFlaggedCount: number;
  /** Phase 0: flagged items used again within 24 h (shadow regret). */
  shadowRegretCount: number;
  /** Phase 0 (P0.8): local hours already sampled for open tabs / windows. */
  sampledHours?: number[];
  /** Phase 0 (P0.8): hourly samples behind `openTabsP50` / `openWindowsP50`. */
  openTabsSamples?: number[];
  openWindowsSamples?: number[];
  /** Phase 0 (P0.8): samples behind `searchToRestoreMsP50`. */
  searchToRestoreSamples?: number[];
}

/** What Insights reads from an item (archived and restored sites). */
export interface MetricItem {
  domain?: string;
  archivedAt?: number;
  restoreCount?: number;
}
