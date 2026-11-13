import type { ReactNode } from 'react';
import { cx } from './cx';

export type ChipTone = 'neutral' | 'accent' | 'warning' | 'danger';

const TONE: Record<ChipTone, string> = {
  neutral: 'bg-surface-2 text-muted border border-line',
  accent: 'bg-chip text-chip-ink',
  warning: 'bg-warning-chip text-warning-chip-ink',
  danger: 'bg-danger-chip text-danger-chip-ink',
};

/** Reason chip: "Unused 61 active h", "Protected: playing audio" (text, never colour alone). */
export function Chip(props: {
  tone?: ChipTone;
  icon?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  const { tone = 'neutral', icon, children, className } = props;
  return (
    <span
      className={cx(
        'inline-flex max-w-full items-center gap-1 rounded-full px-2.5 py-0.5 text-[11px] font-medium whitespace-nowrap',
        TONE[tone],
        className,
      )}
    >
      {icon && <span aria-hidden>{icon}</span>}
      <span className="truncate">{children}</span>
    </span>
  );
}

/** The small "PRO" pill on Pro features. */
export function ProBadge({ className }: { className?: string }) {
  return (
    <span
      className={cx(
        'inline-flex items-center rounded-full border border-accent px-1.5 text-[10px] leading-4 font-semibold tracking-wide text-accent-text',
        className,
      )}
    >
      PRO
    </span>
  );
}
