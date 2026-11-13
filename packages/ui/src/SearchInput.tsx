import { Search, X } from 'lucide-react';
import { type InputHTMLAttributes, type KeyboardEvent, useRef } from 'react';
import { cx } from './cx';

/**
 * Search field: leading icon, clear button. `Esc` clears the text first, then (when
 * empty) calls `onEscape` (the panel closes). `large` is the panel's 20 px input.
 */
export function SearchInput({
  value,
  onValueChange,
  onEscape,
  large = false,
  label,
  className,
  onKeyDown,
  ...rest
}: Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'> & {
  value: string;
  onValueChange: (value: string) => void;
  onEscape?: () => void;
  large?: boolean;
  /** Accessible name. */
  label: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  const keyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    onKeyDown?.(e);
    if (e.defaultPrevented || e.key !== 'Escape') return;
    // Handled only when it does something; otherwise a parent (a dialog) gets it.
    if (value) {
      e.preventDefault();
      onValueChange('');
    } else if (onEscape) {
      e.preventDefault();
      onEscape();
    }
  };
  return (
    <div className={cx('relative flex items-center', className)}>
      <Search
        aria-hidden
        size={large ? 20 : 14}
        className={cx('pointer-events-none absolute text-faint', large ? 'left-4' : 'left-3')}
      />
      <input
        ref={input}
        type="search"
        aria-label={label}
        value={value}
        onChange={(e) => onValueChange(e.target.value)}
        onKeyDown={keyDown}
        className={cx(
          // One focus indicator: the accent border (≥ 3:1) plus a soft ring, not the outline too.
          'w-full rounded-full border border-line bg-surface-2 text-ink placeholder:text-muted focus-visible:border-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-soft [&::-webkit-search-cancel-button]:hidden',
          large ? 'h-12 pr-11 pl-12 text-xl' : 'h-8 pr-8 pl-8 text-xs',
        )}
        {...rest}
      />
      {value && (
        <button
          type="button"
          aria-label="Clear search"
          onClick={() => {
            onValueChange('');
            input.current?.focus();
          }}
          className={cx(
            'absolute flex items-center justify-center rounded-full text-faint hover:text-ink',
            large ? 'right-3 size-7' : 'right-2 size-5',
          )}
        >
          <X aria-hidden size={large ? 16 : 12} />
        </button>
      )}
    </div>
  );
}
