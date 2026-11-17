import { richText } from '@decay/ui';
import { SquarePen } from 'lucide-react';
import { t } from '@/lib/strings';

export interface ProbeBannerProps {
  onAllow: () => void;
  /** One-line layout for the 380 px popup (Chrome caps popups at 600 px tall). */
  compact?: boolean;
  /** Access for some sites only ("On specific sites"): pages elsewhere wait 2×. */
  partial?: boolean;
  busy?: boolean;
}

/**
 * Health banner (Designs/12) shown while Decay has no site access for the form probe
 * (P0.7): web pages then wait twice as long before they're archived (PRD §7.3).
 */
export function ProbeBanner({ onAllow, compact, partial, busy }: ProbeBannerProps) {
  const button = (
    <button
      type="button"
      onClick={onAllow}
      disabled={busy}
      aria-busy={busy || undefined}
      className={`shrink-0 rounded-full border border-line-strong bg-cta font-medium text-cta-ink hover:brightness-95 disabled:opacity-60 ${
        compact ? 'px-3 py-1 text-xs' : 'px-4 py-1.5 text-sm'
      }`}
    >
      {busy ? t('probe.asking') : partial ? t('probe.allowAll') : t('probe.allow')}
    </button>
  );
  if (compact) {
    return (
      <section
        aria-label={t('probe.label')}
        className="relative mt-2 flex items-center gap-2 overflow-hidden rounded-tile border border-line bg-surface py-1.5 pe-1.5 ps-4"
      >
        <span className="absolute inset-y-0 start-0 w-1.5 bg-warning" aria-hidden />
        <p className="min-w-0 flex-1 text-xs leading-snug text-muted">
          {richText((p) => t(partial ? 'probe.compact.partial' : 'probe.compact.none', p), {
            lead: (
              <span className="font-medium text-ink">
                {partial ? t('probe.compact.partialLead') : t('probe.compact.noneLead')}
              </span>
            ),
          })}
        </p>
        {button}
      </section>
    );
  }
  return (
    <section
      aria-label={t('probe.label')}
      className="relative mt-6 flex flex-col gap-3 overflow-hidden rounded-card border border-line bg-surface py-4 pe-5 ps-7 shadow-[var(--shadow-card)] sm:flex-row sm:items-center"
    >
      <span className="absolute inset-y-0 start-0 w-1.5 bg-warning" aria-hidden />
      <div className="flex min-w-0 flex-1 items-start gap-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-full bg-surface-2 text-warning">
          <SquarePen size={16} aria-hidden />
        </span>
        <div className="min-w-0">
          <p className="font-medium">{partial ? t('probe.partial') : t('probe.none')}</p>
          <p className="text-sm text-muted">
            {partial ? t('probe.partial.text') : t('probe.none.text')}
          </p>
        </div>
      </div>
      {/* Lined up with the text (past the icon) on narrow screens. */}
      <div className="ps-12 sm:ps-0">{button}</div>
    </section>
  );
}
