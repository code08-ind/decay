import type { ReactNode } from 'react';
import { cx } from './cx';

export type Health = 'connected' | 'degraded' | 'down';

const HEALTH: Record<Health, { dot: string; text: string }> = {
  connected: { dot: 'bg-accent-dot', text: 'Connected' },
  degraded: { dot: 'bg-warning', text: 'Degraded' },
  down: { dot: 'bg-danger', text: 'Not connected' },
};

/** 8 px status dot with its meaning in words (never colour alone). */
export function StatusDot(props: { status: Health; label?: string; className?: string }) {
  const h = HEALTH[props.status];
  return (
    <span className={cx('inline-flex items-center gap-1.5 text-xs text-muted', props.className)}>
      <span aria-hidden className={cx('size-2 rounded-full', h.dot)} />
      {props.label ?? h.text}
    </span>
  );
}

/** Age stages of an item: fresh, stale, hidden, archived. */
export type AgeStage = 0 | 1 | 2 | 3;
const AGE: Record<AgeStage, { dot: string; text: string }> = {
  0: { dot: 'bg-age-0', text: 'Fresh' },
  1: { dot: 'bg-age-1', text: 'Stale' },
  2: { dot: 'bg-age-2', text: 'Hidden' },
  3: { dot: 'bg-age-3', text: 'Archived' },
};

/** The age-scale dot ("gentle ageing"); the stage name is always available as text. */
export function AgeDot(props: { stage: AgeStage; showLabel?: boolean }) {
  const a = AGE[props.stage];
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-muted">
      <span aria-hidden className={cx('size-2 rounded-full', a.dot)} />
      <span className={props.showLabel ? undefined : 'sr-only'}>{a.text}</span>
    </span>
  );
}

/** White card (22 px radius). */
export function Card(props: { children: ReactNode; className?: string; as?: 'section' | 'div' }) {
  const Tag = props.as ?? 'div';
  return (
    <Tag
      className={cx('rounded-card border border-line bg-surface p-5 shadow-sm', props.className)}
    >
      {props.children}
    </Tag>
  );
}

/** The one deep-green feature card per screen. */
export function FeatureCard(props: { children: ReactNode; className?: string }) {
  return (
    <section
      // Focus rings inside use the card's text colour: the green focus colour vanishes on green.
      className={cx(
        'rounded-card bg-feature p-5 text-feature-ink shadow-sm [--focus:var(--feature-text)]',
        props.className,
      )}
    >
      {props.children}
    </section>
  );
}

/** Empty state: an icon, one line, one action. */
export function EmptyState(props: {
  icon?: ReactNode;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center gap-3 px-6 py-12 text-center">
      {props.icon && (
        <span
          aria-hidden
          className="flex size-12 items-center justify-center rounded-full bg-chip text-chip-ink"
        >
          {props.icon}
        </span>
      )}
      <p className="font-serif text-xl text-ink">{props.title}</p>
      {props.children && <div className="max-w-sm text-sm text-muted">{props.children}</div>}
      {props.action}
    </div>
  );
}
