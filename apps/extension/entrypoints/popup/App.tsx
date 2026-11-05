import { useEffect, useState } from 'react';
import { type Settings, settings } from '@/lib/settings/settings';

// Placeholder until the search popup (P0.6) is built from Designs/*/15-extension-popup-*.
export function App() {
  const [current, setCurrent] = useState<Settings | null>(null);

  useEffect(() => {
    settings.getAll().then(setCurrent);
  }, []);

  return (
    <main style={{ width: 360, padding: 16, fontFamily: 'system-ui, sans-serif' }}>
      <h1 style={{ fontSize: 18, margin: 0 }}>Decay</h1>
      {current && (
        <p style={{ fontSize: 13 }}>
          Mode: <strong>{current.mode}</strong> · Preset: <strong>{current.preset}</strong> (
          {current.thresholds.stale}/{current.thresholds.archive} active h)
        </p>
      )}
    </main>
  );
}
