import { Banner, Button, Chip, EmptyState, FeatureCard, SearchInput, Tabs, tabId } from '@decay/ui';
import { Eye, Ghost, Info, Lock, ShieldCheck, Zap } from 'lucide-react';
import { type ReactNode, useState } from 'react';
import { api, errorMessage, type PanelItem } from '../api';
import { displayUrl } from '../panel/results';
import { t } from '../strings';
import type { PageProps } from './Archive';
import { percent, shadowWeekOver } from './model';
import { ItemTile, useLoad } from './parts';

type List = 'would' | 'kept';

/** Rows whose title or address contains every word of `filter`. */
export function matches(item: Pick<PanelItem, 'title' | 'url'>, filter: string): boolean {
  const hay = `${item.title} ${item.url ?? ''}`.toLowerCase();
  return filter
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}

// Shadow review (Designs/04, F2): what Decay would have archived this week, what it kept
// and why, and the switch to automatic mode.
export function ShadowReview(props: PageProps) {
  const { version, go, notify } = props;
  const report = useLoad(() => api.getShadowReport(), [version]);
  const [list, setList] = useState<List>('would');
  const [filter, setFilter] = useState('');
  const [protectedSites, setProtected] = useState<Set<string>>(new Set());
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const r = report.data;

  const protect = (item: PanelItem) => {
    setError('');
    api
      .protectItem(item.id)
      .then(() => {
        setProtected((old) => new Set(old).add(item.domain ?? item.id));
        notify(t('shadowReview.alwaysKeep', { site: item.domain ?? t('shadowReview.thisSite') }));
      })
      .catch((e) => setError(errorMessage(e)));
  };
  const automatic = () => {
    setBusy(true);
    setError('');
    api
      .setMode('automatic')
      .then(() => {
        notify(t('shadowReview.automaticOn'));
        go('/');
      })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setBusy(false));
  };

  const total = r ? r.tabs + r.windows : 0;
  const ready = r !== null && shadowWeekOver(r.day, r.days);
  // Before the week is over, turning on automatic mode asks once more.
  const [confirming, setConfirming] = useState(false);
  const would = (r?.wouldArchive ?? []).filter((i) => matches(i, filter));
  const kept = (r?.kept ?? []).filter((i) => matches(i, filter));
  return (
    <div className="grid gap-5">
      <div>
        <h1 className="font-serif text-4xl font-normal">{t('shadowReview.title')}</h1>
        <p className="mt-1 text-sm text-muted">
          {r?.mode === 'shadow' ? t('shadowReview.subtitleWeek') : t('shadowReview.subtitle')}
        </p>
      </div>
      {report.error && <Banner tone="danger">{report.error}</Banner>}
      {error && <Banner tone="danger">{error}</Banner>}
      {r && (
        <FeatureCard className="grid gap-5 p-6 lg:grid-cols-[1fr_auto]">
          <div>
            <h2 className="font-serif text-3xl">
              {r.mode !== 'shadow'
                ? t('shadowReview.isAutomatic')
                : ready
                  ? t('shadowReview.ready')
                  : t('shadowReview.day', { day: r.day, days: r.days })}
            </h2>
            <p className="mt-2 text-sm text-feature-sub">
              {total === 0
                ? t('shadowReview.nothingYet')
                : t(r.windows ? 'shadowReview.wouldWindows' : 'shadowReview.would', {
                    tabs: r.tabs,
                    windows: r.windows,
                    reopened: r.reopened,
                    pct: percent(r.reopened, total),
                  })}
            </p>
            <div className="mt-4 grid grid-cols-3 gap-3">
              {[
                [
                  'tabs',
                  t('shadowReview.stat.tabs', { n: r.tabs }),
                  t('shadowReview.stat.simulated'),
                ],
                [
                  'windows',
                  t('shadowReview.stat.windows', { n: r.windows }),
                  t('shadowReview.stat.simulated'),
                ],
                [
                  'regret',
                  t('shadowReview.stat.regret', { pct: percent(r.reopened, total) }),
                  t('shadowReview.stat.usedLater'),
                ],
              ].map(([id, value, sub]) => (
                <div
                  key={id}
                  className="rounded-xl border border-feature-tile-line bg-feature-tile p-3"
                >
                  <p className="font-serif text-2xl">{value}</p>
                  <p className="text-xs text-feature-sub">{sub}</p>
                </div>
              ))}
            </div>
          </div>
          {r.mode === 'shadow' && (
            <div className="flex flex-col justify-center gap-2">
              <Button
                variant="onFeature"
                size="lg"
                icon={<Zap size={16} />}
                loading={busy}
                onClick={ready || confirming ? automatic : () => setConfirming(true)}
              >
                {confirming ? t('shadowReview.confirm') : t('shadowReview.turnOn')}
              </Button>
              {confirming && (
                <p role="status" className="max-w-64 text-xs text-feature-sub">
                  {t('shadowReview.confirm.text', { day: r.day, days: r.days })}
                </p>
              )}
              <Button
                variant="onFeatureOutline"
                size="lg"
                icon={<Eye size={16} />}
                onClick={() => go('/')}
              >
                {t('shadowReview.keepWatching')}
              </Button>
            </div>
          )}
        </FeatureCard>
      )}

      <div className="grid items-start gap-4 xl:grid-cols-[1fr_360px]">
        <section
          aria-label={t('shadowReview.decisions')}
          className="rounded-card border border-line bg-surface p-4 shadow-sm"
        >
          <div className="mb-3 flex flex-wrap items-center gap-3">
            <Tabs<List>
              idPrefix="shadow"
              label={t('shadowReview.decisions')}
              current={list}
              onSelect={setList}
              panelId={(id) => `shadow-${id}`}
              tabs={[
                {
                  id: 'would',
                  label: t('shadowReview.tab.would'),
                  count: r?.wouldArchive.length,
                },
                { id: 'kept', label: t('shadowReview.tab.kept'), count: r?.kept.length },
              ]}
            />
            <SearchInput
              className="ms-auto min-w-48 flex-1"
              label={t('shadowReview.filter')}
              placeholder={t('shadowReview.filterPlaceholder')}
              value={filter}
              onValueChange={setFilter}
              onEscape={() => setFilter('')}
            />
          </div>
          <div id={`shadow-${list}`} role="tabpanel" aria-labelledby={tabId('shadow', list)}>
            {list === 'would' ? (
              would.length === 0 ? (
                <EmptyState
                  icon={<Ghost size={20} />}
                  title={filter ? t('shadowReview.noMatch') : t('shadowReview.nothing')}
                >
                  {filter ? t('shadowReview.tryOther') : t('shadowReview.wouldEmpty')}
                </EmptyState>
              ) : (
                <ul className="divide-y divide-line">
                  {would.map((i) => {
                    const kept = protectedSites.has(i.domain ?? i.id);
                    return (
                      <Row key={i.id} item={i}>
                        {i.reason && <Chip>{i.reason}</Chip>}
                        {i.usedAfter && <Chip tone="warning">{t('shadowReview.usedAgain')}</Chip>}
                        {!i.live && <Chip>{t('shadowReview.closedSince')}</Chip>}
                        {i.domain &&
                          (kept ? (
                            <Chip tone="accent" icon={<ShieldCheck size={12} />}>
                              {t('shadowReview.alwaysKept')}
                            </Chip>
                          ) : (
                            <Button
                              size="sm"
                              icon={<ShieldCheck size={12} />}
                              aria-label={t('shadowReview.protectSite', { site: i.domain })}
                              onClick={() => protect(i)}
                            >
                              {t('shadowReview.protect')}
                            </Button>
                          ))}
                      </Row>
                    );
                  })}
                </ul>
              )
            ) : kept.length === 0 ? (
              <EmptyState icon={<ShieldCheck size={20} />} title={t('shadowReview.keptEmpty')}>
                {t('shadowReview.keptEmpty.text')}
              </EmptyState>
            ) : (
              <ul className="divide-y divide-line">
                {kept.map((i) => (
                  <Row key={i.id} item={i}>
                    <Chip>{t('shadowReview.unused', { n: i.activeHours })}</Chip>
                    <Chip tone="accent">{t('shadowReview.protectedBy', { why: i.label })}</Chip>
                  </Row>
                ))}
              </ul>
            )}
          </div>
          <p className="mt-3 text-xs text-muted">
            {list === 'would'
              ? t('shadowReview.showingWould', {
                  shown: would.length,
                  total: r?.wouldArchive.length ?? 0,
                })
              : t('shadowReview.showingKept', { shown: kept.length, total: r?.kept.length ?? 0 })}
          </p>
        </section>

        <aside className="rounded-card border border-line bg-surface p-5 shadow-sm">
          <h2 className="font-serif text-xl">{t('shadowReview.why')}</h2>
          <p className="mt-1 text-[13px] text-muted">{t('shadowReview.why.text')}</p>
          <ul className="mt-4 grid gap-2">
            {(r?.keptBy ?? []).map((k) => (
              <li
                key={k.by}
                className="flex items-center justify-between rounded-xl border border-line bg-surface-2 px-3 py-2.5 text-sm"
              >
                {k.label}
                <span className="grid min-w-6 place-items-center rounded-full border border-line px-1.5 text-xs">
                  {k.count}
                </span>
              </li>
            ))}
            {r && r.keptBy.length === 0 && (
              <li className="text-sm text-muted">{t('shadowReview.why.none')}</li>
            )}
          </ul>
          {r?.mode === 'shadow' && (
            <p className="mt-4 flex gap-2 rounded-xl bg-surface-2 p-3 text-[13px] text-muted">
              <Info size={14} aria-hidden className="mt-0.5 shrink-0" />
              {t('shadowReview.keepsGoing')}
            </p>
          )}
          <p className="mt-4 flex items-center gap-1.5 border-t border-line pt-3 text-xs text-muted">
            <Lock size={12} aria-hidden /> {t('shadowReview.local')}
          </p>
        </aside>
      </div>
    </div>
  );
}

function Row(props: { item: PanelItem; children: ReactNode }) {
  const i = props.item;
  return (
    <li className="flex flex-wrap items-center gap-3 py-3">
      <ItemTile item={i} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">{i.title || t('item.untitled')}</span>
        <span className="block truncate font-mono text-xs text-muted">
          {i.kind === 'window' ? t('item.appWindow', { app: i.appName }) : displayUrl(i.url)}
        </span>
      </span>
      <span className="flex flex-wrap items-center justify-end gap-2">{props.children}</span>
    </li>
  );
}
