import { Banner, Button, Chip, EmptyState, FeatureCard, SearchInput, Tabs, tabId } from '@decay/ui';
import { Eye, Ghost, Info, Lock, ShieldCheck, Zap } from 'lucide-react';
import { type ReactNode, useState } from 'react';
import { api, errorMessage, type PanelItem } from '../api';
import { displayUrl } from '../panel/results';
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
        notify(`Decay will always keep ${item.domain ?? 'this site'}`);
      })
      .catch((e) => setError(errorMessage(e)));
  };
  const automatic = () => {
    setBusy(true);
    setError('');
    api
      .setMode('automatic')
      .then(() => {
        notify('Automatic mode is on. Undo any sweep from Sweeps.');
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
        <h1 className="font-serif text-4xl font-normal">Shadow review</h1>
        <p className="mt-1 text-sm text-muted">
          Here's what Decay would have done{r?.mode === 'shadow' ? ' this week' : ''}. Nothing has
          been closed.
        </p>
      </div>
      {report.error && <Banner tone="danger">{report.error}</Banner>}
      {error && <Banner tone="danger">{error}</Banner>}
      {r && (
        <FeatureCard className="grid gap-5 p-6 lg:grid-cols-[1fr_auto]">
          <div>
            <h2 className="font-serif text-3xl">
              {r.mode !== 'shadow'
                ? 'Automatic mode is on'
                : ready
                  ? "You're ready for automatic mode"
                  : `Day ${r.day} of ${r.days} in shadow mode`}
            </h2>
            <p className="mt-2 text-sm text-feature-sub">
              {total === 0
                ? 'Decay hasn’t found anything to archive yet. It needs a few hours of your active time.'
                : `Decay would have archived ${r.tabs} ${r.tabs === 1 ? 'tab' : 'tabs'}${
                    r.windows ? ` and ${r.windows} ${r.windows === 1 ? 'window' : 'windows'}` : ''
                  }. You used ${r.reopened} of them again afterwards (${percent(r.reopened, total)}).`}
            </p>
            <div className="mt-4 grid grid-cols-3 gap-3">
              {[
                [`${r.tabs}`, r.tabs === 1 ? 'tab' : 'tabs', 'simulated archive'],
                [`${r.windows}`, r.windows === 1 ? 'window' : 'windows', 'simulated archive'],
                [percent(r.reopened, total), 'regret', 'used again later'],
              ].map(([n, what, sub]) => (
                <div
                  key={sub + (what ?? '')}
                  className="rounded-xl border border-feature-tile-line bg-feature-tile p-3"
                >
                  <p className="font-serif text-2xl">
                    {n} {what}
                  </p>
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
                {confirming ? 'Yes, turn it on now' : 'Turn on automatic mode'}
              </Button>
              {confirming && (
                <p role="status" className="max-w-64 text-xs text-feature-sub">
                  It's day {r.day} of {r.days}. Decay will start closing stale tabs; any sweep can
                  be undone from Sweeps.
                </p>
              )}
              <Button
                variant="onFeatureOutline"
                size="lg"
                icon={<Eye size={16} />}
                onClick={() => go('/')}
              >
                Keep watching
              </Button>
            </div>
          )}
        </FeatureCard>
      )}

      <div className="grid items-start gap-4 xl:grid-cols-[1fr_360px]">
        <section
          aria-label="Decisions"
          className="rounded-card border border-line bg-surface p-4 shadow-sm"
        >
          <div className="mb-3 flex flex-wrap items-center gap-3">
            <Tabs<List>
              idPrefix="shadow"
              label="Decisions"
              current={list}
              onSelect={setList}
              panelId={(id) => `shadow-${id}`}
              tabs={[
                { id: 'would', label: 'Would archive', count: r?.wouldArchive.length },
                { id: 'kept', label: 'Protected', count: r?.kept.length },
              ]}
            />
            <SearchInput
              className="ml-auto min-w-48 flex-1"
              label="Filter"
              placeholder="Filter…"
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
                  title={filter ? 'Nothing matches' : 'Nothing yet'}
                >
                  {filter ? 'Try other words.' : 'Tabs Decay would archive show up here.'}
                </EmptyState>
              ) : (
                <ul className="divide-y divide-line">
                  {would.map((i) => {
                    const kept = protectedSites.has(i.domain ?? i.id);
                    return (
                      <Row key={i.id} item={i}>
                        {i.reason && <Chip>{i.reason}</Chip>}
                        {i.usedAfter && <Chip tone="warning">Used again</Chip>}
                        {!i.live && <Chip>Closed since</Chip>}
                        {i.domain &&
                          (kept ? (
                            <Chip tone="accent" icon={<ShieldCheck size={12} />}>
                              Always kept
                            </Chip>
                          ) : (
                            <Button
                              size="sm"
                              icon={<ShieldCheck size={12} />}
                              aria-label={`Protect ${i.domain}`}
                              onClick={() => protect(i)}
                            >
                              Protect
                            </Button>
                          ))}
                      </Row>
                    );
                  })}
                </ul>
              )
            ) : kept.length === 0 ? (
              <EmptyState icon={<ShieldCheck size={20} />} title="Nothing protected right now">
                Pinned, playing and unsaved tabs that are old enough to archive show up here.
              </EmptyState>
            ) : (
              <ul className="divide-y divide-line">
                {kept.map((i) => (
                  <Row key={i.id} item={i}>
                    <Chip>Unused {i.activeHours} active h</Chip>
                    <Chip tone="accent">Protected: {i.label}</Chip>
                  </Row>
                ))}
              </ul>
            )}
          </div>
          <p className="mt-3 text-xs text-muted">
            {list === 'would'
              ? `Showing ${would.length} of ${r?.wouldArchive.length ?? 0} simulated decisions`
              : `Showing ${kept.length} of ${r?.kept.length ?? 0} protected tabs`}
          </p>
        </section>

        <aside className="rounded-card border border-line bg-surface p-5 shadow-sm">
          <h2 className="font-serif text-xl">Why things were kept</h2>
          <p className="mt-1 text-[13px] text-muted">
            Decay leaves tabs alone while one of your safety rules applies.
          </p>
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
              <li className="text-sm text-muted">Nothing is protected right now.</li>
            )}
          </ul>
          {r?.mode === 'shadow' && (
            <p className="mt-4 flex gap-2 rounded-xl bg-surface-2 p-3 text-[13px] text-muted">
              <Info size={14} aria-hidden className="mt-0.5 shrink-0" />
              Shadow mode keeps going until you turn on automatic mode. Nothing is closed before
              then.
            </p>
          )}
          <p className="mt-4 flex items-center gap-1.5 border-t border-line pt-3 text-xs text-muted">
            <Lock size={12} aria-hidden /> Local processing only
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
        <span className="block truncate text-sm font-medium">{i.title || 'Untitled'}</span>
        <span className="block truncate font-mono text-xs text-muted">
          {i.kind === 'window' ? `${i.appName} window` : displayUrl(i.url)}
        </span>
      </span>
      <span className="flex flex-wrap items-center justify-end gap-2">{props.children}</span>
    </li>
  );
}
