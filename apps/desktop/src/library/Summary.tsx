import { Banner, Button, richText } from '@decay/ui';
import { AppWindow, History, PanelsTopLeft, RotateCcw, ShieldCheck, Timer } from 'lucide-react';
import { useState } from 'react';
import { api, type DailySummary, restoreChoices } from '../api';
import { displayUrl } from '../panel/results';
import { t } from '../strings';
import type { PageProps } from './Archive';
import { yesterday } from './model';
import { ItemTile, type RestoreChoices, RestoreElsewhere, Stat, useLoad } from './parts';

type Hours = { stale: number; hidden: number; archive: number };
const MAX_H = 720;

/** "Keep longer": twice the current pace, kept in 1–720 h and increasing; `null` when the
 * pace is already near the limit (nothing longer to offer). */
export function longerHours(t: Hours): Hours | null {
  const archive = Math.min(MAX_H, Math.round(t.archive * 2));
  const hidden = Math.min(archive - 1, Math.round(t.hidden * 2));
  const stale = Math.min(hidden - 1, Math.round(t.stale * 2));
  if (archive <= t.archive || stale < 1) return null;
  return { stale, hidden, archive };
}

// Yesterday (Designs/11, F10): what moved, what's worth a look, and the archived items
// with a one-click restore. The morning notification says the same in one line.
export function Summary(props: PageProps) {
  const { version, run, notify, refresh } = props;
  // A refused restore (its profile isn't open) offers the connected ones (P2.8).
  const [choices, setChoices] = useState<RestoreChoices | null>(null);
  const restore = (call: () => Promise<unknown>) =>
    run(async () => {
      setChoices(null);
      try {
        await call();
      } catch (e) {
        setChoices(restoreChoices(e));
        throw e;
      }
      notify(t('summary.restoring'));
    });
  const [from, to] = yesterday(Date.now());
  const summary = useLoad(() => api.getDailySummary(from, to), [version, from]);
  const settings = useLoad(() => api.getSettings(), [version]);
  const s: DailySummary | null = summary.data;
  const thresholds = (settings.data?.thresholds as Hours | undefined) ?? {
    stale: 12,
    hidden: 36,
    archive: 60,
  };
  const longer = longerHours(thresholds);
  const hints = (s?.hints ?? []).filter((h) => h.kind === 'alwaysKeep' || longer !== null);
  return (
    <section aria-labelledby="summary-h" className="grid gap-4">
      <div>
        <h1 id="summary-h" className="font-serif text-4xl font-normal">
          {t('summary.title')}
        </h1>
        <p className="text-sm text-muted">{t('summary.subtitle')}</p>
      </div>
      {summary.error && <Banner tone="danger">{summary.error}</Banner>}
      {choices && (
        <RestoreElsewhere
          choices={choices}
          onPick={(c) => restore(() => api.restoreItemsIn(choices.itemIds, c.appId, c.profileId))}
        />
      )}
      {s && (
        <>
          <div className="grid gap-4 sm:grid-cols-3">
            <Stat
              label={t('summary.tabs')}
              value={s.archivedTabs}
              icon={<PanelsTopLeft size={16} />}
            />
            <Stat
              label={t('summary.windows')}
              value={s.hiddenWindows}
              icon={<AppWindow size={16} />}
            />
            <Stat label={t('summary.reopened')} value={s.restored} icon={<History size={16} />} />
          </div>
          {hints.length > 0 && (
            <section
              aria-labelledby="hints-h"
              className="rounded-card border border-line bg-surface p-5 shadow-sm"
            >
              <h2
                id="hints-h"
                className="mb-3 text-[11px] font-semibold tracking-wider text-muted uppercase"
              >
                {t('summary.hints')}
              </h2>
              <ul className="grid gap-2">
                {hints.map((h) => (
                  <li key={`${h.kind}-${h.domain}`} className="flex flex-wrap items-center gap-3">
                    <span className="min-w-0 flex-1 text-sm">
                      {richText(
                        (p) =>
                          t(
                            h.kind === 'alwaysKeep' ? 'summary.hint.keep' : 'summary.hint.longer',
                            p,
                          ),
                        { site: <strong>{h.domain}</strong>, n: h.count },
                      )}
                    </span>
                    {h.kind === 'alwaysKeep' ? (
                      <Button
                        size="sm"
                        icon={<ShieldCheck size={14} aria-hidden />}
                        aria-label={t('summary.alwaysKeepFor', { site: h.domain })}
                        onClick={() =>
                          run(async () => {
                            await api.createRule({
                              scope: 'domain',
                              match: h.domain,
                              action: 'protect',
                            });
                            notify(t('summary.alwaysKept', { site: h.domain }));
                            refresh();
                          })
                        }
                      >
                        {t('summary.alwaysKeep')}
                      </Button>
                    ) : (
                      <Button
                        size="sm"
                        icon={<Timer size={14} aria-hidden />}
                        aria-label={t('summary.keepLongerFor', { site: h.domain })}
                        onClick={() =>
                          run(async () => {
                            if (!longer) return;
                            await api.createRule({
                              scope: 'domain',
                              match: h.domain,
                              action: 'customThreshold',
                              thresholdHours: longer,
                            });
                            notify(t('summary.keptLonger', { site: h.domain }));
                            refresh();
                          })
                        }
                      >
                        {t('summary.keepLonger')}
                      </Button>
                    )}
                  </li>
                ))}
              </ul>
            </section>
          )}
          <section
            aria-labelledby="archived-h"
            className="rounded-card border border-line bg-surface shadow-sm"
          >
            <h2
              id="archived-h"
              className="border-b border-line px-5 py-3 text-[11px] font-semibold tracking-wider text-muted uppercase"
            >
              {t('summary.archived')}
            </h2>
            {s.items.length === 0 ? (
              <p className="px-5 py-6 text-sm text-muted">{t('summary.empty')}</p>
            ) : (
              <ul className="divide-y divide-line">
                {s.items.map((i) => (
                  <li key={i.id} className="flex items-center gap-3 px-5 py-2.5">
                    <ItemTile item={i} size="sm" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm">{i.title || displayUrl(i.url)}</span>
                      <span className="block truncate text-xs text-muted">{displayUrl(i.url)}</span>
                    </span>
                    <Button
                      size="sm"
                      icon={<RotateCcw className="rtl:-scale-x-100" size={14} aria-hidden />}
                      aria-label={t('summary.restoreItem', { title: i.title || displayUrl(i.url) })}
                      onClick={() => restore(() => api.restoreItems([i.id]))}
                    >
                      {t('summary.restore')}
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </>
      )}
    </section>
  );
}
