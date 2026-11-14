// Pieces the Settings tabs share.
import { listen } from '@tauri-apps/api/event';
import { type ReactNode, useEffect, useState } from 'react';
import { api, errorMessage } from '../api';

export type SettingsMap = Record<string, unknown>;

export interface TabProps {
  settings: SettingsMap | null;
  /** Saves a change; the screen follows `settings:changed`. Shows its error. */
  save: (patch: SettingsMap, done?: string) => void;
  /** A short confirmation ("Saved"). */
  notify: (text: string) => void;
  fail: (e: unknown) => void;
}

/** All settings, kept current by `settings:changed` (from any window). */
export function useSettings(onError: (e: unknown) => void): SettingsMap | null {
  const [settings, setSettings] = useState<SettingsMap | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: subscribe once
  useEffect(() => {
    let changed = false;
    const off = listen<SettingsMap>('settings:changed', (e) => {
      changed = true;
      setSettings(e.payload);
    });
    api
      .getSettings()
      .then((s) => changed || setSettings(s))
      .catch(onError);
    return () => {
      off.then((f) => f()).catch(() => {});
    };
  }, []);
  return settings;
}

/** A titled card of rows (Designs/09). */
export function Group(props: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-card border border-line bg-surface p-5 shadow-sm">
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h2 className="text-[11px] font-semibold tracking-wider text-muted uppercase">
          {props.title}
        </h2>
        {props.aside && <span className="text-xs text-muted">{props.aside}</span>}
      </div>
      <div className="grid divide-y divide-line">{props.children}</div>
    </section>
  );
}

/** One setting: its name and explanation, and its control. */
export function Row(props: {
  label: string;
  description?: ReactNode;
  children?: ReactNode;
  id?: string;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-4 py-3 first:pt-0 last:pb-0">
      <div className="min-w-0 flex-1">
        <p id={props.id} className="text-sm font-medium">
          {props.label}
        </p>
        {props.description && <p className="text-[13px] text-muted">{props.description}</p>}
      </div>
      {props.children}
    </div>
  );
}

/** The heading of a tab. */
export function TabHeader(props: { title: string; children?: ReactNode }) {
  return (
    <header className="mb-5">
      <h1 className="font-serif text-4xl font-normal">{props.title}</h1>
      {props.children && <p className="mt-1 text-sm text-muted">{props.children}</p>}
    </header>
  );
}

export { errorMessage };
