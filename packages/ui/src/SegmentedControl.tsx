import { type KeyboardEvent, useRef } from 'react';
import { cx } from './cx';
import { logicalArrow } from './i18n';

export interface Segment<T extends string> {
  value: T;
  label: string;
}

/**
 * Segmented pills (presets Gentle / Balanced / Aggressive / Custom). A radio group:
 * arrow keys move and select, Tab enters at the selected option.
 */
export function SegmentedControl<T extends string>(props: {
  label: string;
  options: Segment<T>[];
  value: T;
  onChange: (value: T) => void;
  className?: string;
}) {
  const { label, options, value, onChange, className } = props;
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: KeyboardEvent, index: number) => {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[logicalArrow(e.key)];
    const edge = { Home: 0, End: options.length - 1 }[e.key];
    if (step === undefined && edge === undefined) return;
    e.preventDefault();
    const next = edge ?? (index + (step ?? 0) + options.length) % options.length;
    const option = options[next];
    if (!option) return;
    onChange(option.value);
    refs.current[next]?.focus();
  };
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cx(
        'inline-flex items-center rounded-full border border-line bg-surface-2 p-1',
        className,
      )}
    >
      {options.map((o, i) => {
        const selected = o.value === value;
        return (
          // biome-ignore lint/a11y/useSemanticElements: pill buttons with the radio pattern (roving tabindex, arrow keys)
          <button
            key={o.value}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(o.value)}
            onKeyDown={(e) => onKey(e, i)}
            className={cx(
              'rounded-full px-3.5 py-1 text-xs transition-colors',
              selected
                ? 'bg-primary font-semibold text-primary-ink shadow-sm'
                : 'font-medium text-muted hover:text-ink',
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
