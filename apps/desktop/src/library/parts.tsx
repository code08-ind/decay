// Small pieces the Library screens share.
import { cx } from '@decay/ui';
import { AppWindow } from 'lucide-react';
import { type DependencyList, type ReactNode, useEffect, useState } from 'react';
import { errorMessage, type PanelItem } from '../api';
import { monogram } from '../panel/results';

const TINT = {
  1: 'bg-avatar-1 text-avatar-1-ink',
  2: 'bg-avatar-2 text-avatar-2-ink',
  3: 'bg-avatar-3 text-avatar-3-ink',
  4: 'bg-avatar-4 text-avatar-4-ink',
} as const;

/** A favicon stand-in: a letter tile per site, a window icon for windows. */
export function ItemTile(props: {
  item: Pick<PanelItem, 'domain' | 'title' | 'kind'>;
  size?: 'sm' | 'md';
}) {
  const size = props.size === 'sm' ? 'size-7 text-xs' : 'size-9 text-sm';
  if (props.item.kind === 'window') {
    return (
      <span
        aria-hidden
        className={cx('grid shrink-0 place-items-center rounded-lg bg-surface-3 text-muted', size)}
      >
        <AppWindow size={16} />
      </span>
    );
  }
  const m = monogram(props.item);
  return (
    <span
      aria-hidden
      className={cx(
        'grid shrink-0 place-items-center rounded-lg font-semibold',
        TINT[m.tint],
        size,
      )}
    >
      {m.letter}
    </span>
  );
}

/** One stat card (Designs/05). */
export function Stat(props: {
  label: string;
  value: ReactNode;
  detail?: ReactNode;
  icon: ReactNode;
}) {
  return (
    <section className="rounded-card border border-line bg-surface p-4 shadow-sm">
      <div className="flex items-start justify-between">
        <h2 className="text-[13px] text-muted">{props.label}</h2>
        <span
          aria-hidden
          className="grid size-8 place-items-center rounded-full bg-surface-2 text-muted"
        >
          {props.icon}
        </span>
      </div>
      <p className="mt-2 font-serif text-4xl leading-none">{props.value}</p>
      {props.detail && <p className="mt-2 text-xs text-muted">{props.detail}</p>}
    </section>
  );
}

export interface Loaded<T> {
  data: T | null;
  error: string;
  loading: boolean;
}

/**
 * Loads `load()` and again whenever `deps` change; an older answer that arrives after a
 * newer request is dropped.
 */
export function useLoad<T>(load: () => Promise<T>, deps: DependencyList): Loaded<T> {
  const [state, setState] = useState<Loaded<T>>({ data: null, error: '', loading: true });
  // The caller's deps decide when to load again (compared by value).
  const key = JSON.stringify(deps);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands for the caller's deps
  useEffect(() => {
    let live = true;
    setState((s) => ({ ...s, loading: true }));
    load()
      .then((data) => live && setState({ data, error: '', loading: false }))
      .catch((e) => live && setState((s) => ({ ...s, error: errorMessage(e), loading: false })));
    return () => {
      live = false;
    };
  }, [key]);
  return state;
}
