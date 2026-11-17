import { useId } from 'react';
import { cx } from './cx';

/** On/off switch with its label (and optional description) — a native-feeling toggle. */
export function Switch(props: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
  className?: string;
}) {
  const { checked, onChange, label, description, disabled, className } = props;
  const id = useId();
  return (
    <div className={cx('flex items-center justify-between gap-6', className)}>
      <div className="min-w-0">
        <label htmlFor={id} className="text-sm font-medium text-ink">
          {label}
        </label>
        {description && (
          <p id={`${id}-d`} className="text-xs text-muted">
            {description}
          </p>
        )}
      </div>
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={checked}
        aria-describedby={description ? `${id}-d` : undefined}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={cx(
          'relative h-6 w-10 shrink-0 rounded-full border transition-colors disabled:opacity-50',
          // Off: a muted outline and knob (≥ 3:1 against the card), not a faint track.
          checked ? 'border-accent bg-accent' : 'border-muted bg-surface-3',
        )}
      >
        <span
          aria-hidden
          className={cx(
            'absolute top-0.5 start-0.5 size-[18px] rounded-full shadow-sm transition-transform',
            checked ? 'translate-x-4 rtl:-translate-x-4 bg-cta' : 'bg-muted',
          )}
        />
      </button>
    </div>
  );
}
