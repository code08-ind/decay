import { CircleAlert, Info, PauseCircle, ShieldAlert } from 'lucide-react';
import type { ReactNode } from 'react';
import { cx } from './cx';

export type BannerTone = 'info' | 'success' | 'warning' | 'danger';

const TONE: Record<BannerTone, { box: string; chip: string; icon: ReactNode }> = {
  info: {
    box: 'bg-surface border-line',
    chip: 'bg-surface-2 text-muted',
    icon: <Info size={16} />,
  },
  success: {
    box: 'bg-accent-soft border-line',
    chip: 'bg-chip text-chip-ink',
    icon: <PauseCircle size={16} />,
  },
  warning: {
    box: 'bg-warning-soft border-line',
    chip: 'bg-warning-chip text-warning-chip-ink',
    icon: <CircleAlert size={16} />,
  },
  danger: {
    box: 'bg-danger-soft border-line',
    chip: 'bg-danger-chip text-danger-chip-ink',
    icon: <ShieldAlert size={16} />,
  },
};

/**
 * Health / status banner (Designs/12): full-width card, round icon chip, one message,
 * one fix-it action. Danger banners are announced right away (`role="alert"`).
 */
export function Banner(props: {
  tone?: BannerTone;
  icon?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  const { tone = 'info', icon, children, action, className } = props;
  const t = TONE[tone];
  return (
    <div
      role={tone === 'danger' ? 'alert' : 'status'}
      className={cx(
        'flex min-h-16 items-center justify-between gap-4 rounded-card border px-5 py-3 shadow-sm',
        t.box,
        className,
      )}
    >
      <div className="flex min-w-0 items-center gap-3.5">
        <span
          aria-hidden
          className={cx('flex size-9 shrink-0 items-center justify-center rounded-full', t.chip)}
        >
          {icon ?? t.icon}
        </span>
        <div className="min-w-0 text-sm font-medium text-ink">{children}</div>
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
  );
}
