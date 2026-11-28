import type { Mode } from '@/lib/db/types';
import { type Key, t } from '@/lib/strings';
import { DecayLeaf } from './DecayLeaf';

const MODE_LABEL: Record<Mode, Key> = {
  shadow: 'mode.shadow',
  automatic: 'mode.automatic',
  paused: 'mode.paused',
};

export type PageId = 'review' | 'sweeps' | 'insights';

const PAGES: { id: PageId; href: string; label: Key }[] = [
  { id: 'review', href: 'review.html', label: 'pages.review' },
  { id: 'sweeps', href: 'sweeps.html', label: 'pages.sweeps' },
  { id: 'insights', href: 'insights.html', label: 'pages.insights' },
];

/** Shared header of the extension pages: wordmark, links between the pages, current mode. */
export function PageHeader(props: { current: PageId; mode: Mode }) {
  return (
    <header className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b border-line px-4 py-3 sm:px-10">
      <div className="flex items-center gap-2">
        <DecayLeaf size={20} className="text-accent" aria-hidden />
        <span className="font-serif text-xl">Decay</span>
      </div>
      <nav aria-label={t('pages.nav')} className="order-last w-full sm:order-none sm:w-auto">
        <ul className="flex gap-1 text-sm">
          {PAGES.map((p) => (
            <li key={p.id}>
              <a
                href={p.href}
                aria-current={p.id === props.current ? 'page' : undefined}
                className={`block rounded-full px-3 py-1 focus-visible:outline-2 focus-visible:outline-accent ${
                  p.id === props.current
                    ? 'bg-surface-2 font-medium text-ink'
                    : 'text-muted hover:text-ink'
                }`}
              >
                {t(p.label)}
              </a>
            </li>
          ))}
        </ul>
      </nav>
      <span className="inline-flex items-center gap-2 rounded-full border border-line bg-surface px-3 py-1 text-sm">
        <span className="size-2 rounded-full bg-accent-dot" aria-hidden />
        {t(MODE_LABEL[props.mode])}
      </span>
    </header>
  );
}
