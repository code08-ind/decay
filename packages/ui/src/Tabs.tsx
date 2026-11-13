import { type KeyboardEvent, type ReactNode, useId, useRef } from 'react';
import { cx } from './cx';

export interface Tab<T extends string> {
  id: T;
  label: string;
  icon?: ReactNode;
  /** A count shown after the label. */
  count?: number;
}

/**
 * The pill tab bar at the top of a window (Sage direction: no sidebar). A tablist:
 * arrow keys move focus and select, Home / End jump. Tab ids are `tabId(idPrefix, id)`
 * (unique per tablist); the selected tab controls `panelId(id)`, which the page renders
 * with `aria-labelledby={tabId(idPrefix, current)}`.
 */
/** The element id of a tab (for its panel's `aria-labelledby`). */
export function tabId(prefix: string, id: string): string {
  return `${prefix}-tab-${id}`;
}

export function Tabs<T extends string>(props: {
  label: string;
  /** Prefix for the tab ids (default: unique per instance). */
  idPrefix?: string;
  tabs: Tab<T>[];
  current: T;
  onSelect: (id: T) => void;
  panelId?: (id: T) => string;
  className?: string;
}) {
  const { label, tabs, current, onSelect, panelId, className } = props;
  const generated = useId();
  const prefix = props.idPrefix ?? generated;
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: KeyboardEvent, index: number) => {
    const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
    const edge = { Home: 0, End: tabs.length - 1 }[e.key];
    if (step === undefined && edge === undefined) return;
    e.preventDefault();
    const next = edge ?? (index + (step ?? 0) + tabs.length) % tabs.length;
    const tab = tabs[next];
    if (!tab) return;
    onSelect(tab.id);
    refs.current[next]?.focus();
  };
  return (
    <div
      role="tablist"
      aria-label={label}
      className={cx('inline-flex items-center gap-1 rounded-full bg-surface-2 p-1', className)}
    >
      {tabs.map((t, i) => {
        const selected = t.id === current;
        return (
          <button
            key={t.id}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="tab"
            id={tabId(prefix, t.id)}
            aria-selected={selected}
            // Only the selected tab's panel exists.
            aria-controls={selected ? panelId?.(t.id) : undefined}
            tabIndex={selected ? 0 : -1}
            onClick={() => onSelect(t.id)}
            onKeyDown={(e) => onKey(e, i)}
            className={cx(
              'inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[13px] transition-all',
              selected
                ? 'bg-surface font-semibold text-ink shadow-sm'
                : 'text-muted hover:text-ink',
            )}
          >
            {t.icon && <span aria-hidden>{t.icon}</span>}
            {t.label}
            {t.count !== undefined && <span className="text-xs text-muted">{t.count}</span>}
          </button>
        );
      })}
    </div>
  );
}
