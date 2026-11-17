import type { ButtonHTMLAttributes, ReactNode, Ref } from 'react';
import { cx } from './cx';

export type ButtonVariant =
  | 'primary'
  | 'secondary'
  | 'ghost'
  | 'danger'
  /** White pill on the green feature card. */
  | 'onFeature'
  /** Outlined pill on the green feature card. */
  | 'onFeatureOutline';
export type ButtonSize = 'sm' | 'md' | 'lg';

const VARIANT: Record<ButtonVariant, string> = {
  primary: 'bg-primary text-primary-ink shadow-sm hover:opacity-90',
  secondary:
    'bg-cta text-cta-ink border border-line-strong shadow-sm hover:bg-surface-2 hover:text-ink',
  ghost: 'text-muted hover:text-ink hover:bg-surface-2',
  danger: 'bg-danger text-primary-ink shadow-sm hover:opacity-90',
  onFeature: 'bg-cta text-cta-ink shadow-sm hover:opacity-90',
  onFeatureOutline: 'border border-feature-tile-line text-feature-ink hover:bg-feature-tile',
};

// Heights 28 / 32 / 40 (UI Design Guidelines §7).
const SIZE: Record<ButtonSize, string> = {
  sm: 'h-7 px-3 text-xs gap-1',
  md: 'h-8 px-4 text-[13px] gap-1.5',
  lg: 'h-10 px-5 text-sm gap-2',
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Leading icon (decorative). */
  icon?: ReactNode;
  /** Busy: disabled, with a spinner and `aria-busy`. */
  loading?: boolean;
  /** Shortcut hint, e.g. "⌘↵": shown as a tooltip and announced. */
  shortcut?: string;
  /** The button element (React 19 passes it as a prop). */
  ref?: Ref<HTMLButtonElement>;
}

/** Pill button (Sage direction: black primary, white secondary). */
export function Button({
  variant = 'secondary',
  size = 'md',
  icon,
  loading = false,
  shortcut,
  className,
  children,
  disabled,
  type = 'button',
  title,
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      aria-keyshortcuts={shortcut}
      title={title ?? (shortcut ? shortcut : undefined)}
      className={cx(
        'inline-flex shrink-0 items-center justify-center rounded-full font-medium whitespace-nowrap transition active:scale-[0.97] disabled:pointer-events-none disabled:opacity-50 aria-disabled:cursor-not-allowed aria-disabled:opacity-50 aria-disabled:active:scale-100',
        VARIANT[variant],
        SIZE[size],
        className,
      )}
      {...rest}
    >
      {loading ? <Spinner /> : icon ? <span aria-hidden>{icon}</span> : null}
      {children}
    </button>
  );
}

/** A small busy indicator (decorative; pair it with `aria-busy` or text). */
export function Spinner({ className }: { className?: string }) {
  return (
    <span
      aria-hidden
      className={cx(
        'inline-block size-3.5 animate-spin rounded-full border-2 border-current border-e-transparent',
        className,
      )}
    />
  );
}

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** What the button does (its accessible name). */
  label: string;
  icon: ReactNode;
  size?: 'sm' | 'md' | 'lg';
  /** Bordered surface circle (toolbars) or plain (rows). */
  outlined?: boolean;
  /** On the green feature card (its text colour, a tile-coloured hover). */
  onFeature?: boolean;
}

/** Round icon-only button; `label` is required, so it's never unnamed. */
export function IconButton({
  label,
  icon,
  size = 'md',
  outlined = false,
  onFeature = false,
  className,
  type = 'button',
  ...rest
}: IconButtonProps) {
  const dim = { sm: 'size-7', md: 'size-8', lg: 'size-10' }[size];
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      className={cx(
        'inline-flex shrink-0 items-center justify-center rounded-full transition disabled:pointer-events-none disabled:opacity-50',
        onFeature
          ? 'text-feature-ink hover:bg-feature-tile'
          : outlined
            ? 'border border-line bg-surface text-muted shadow-sm hover:bg-surface-2 hover:text-ink'
            : 'text-muted hover:bg-surface-2 hover:text-ink',
        dim,
        className,
      )}
      {...rest}
    >
      <span aria-hidden>{icon}</span>
    </button>
  );
}
