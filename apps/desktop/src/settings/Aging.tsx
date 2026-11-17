import { Button, FeatureCard, SegmentedControl } from '@decay/ui';
import { ArrowRight } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, type Thresholds } from '../api';
import { type Key, t } from '../strings';
import { BALANCED, PRESET_HOURS, paceOf, thresholdsProblem, workTime } from './model';
import { Group, TabHeader, type TabProps } from './parts';

type Pace = 'gentle' | 'balanced' | 'aggressive' | 'custom';

function readThresholds(v: unknown): Thresholds {
  const h = v as Partial<Thresholds> | null;
  return h &&
    typeof h.stale === 'number' &&
    typeof h.hidden === 'number' &&
    typeof h.archive === 'number'
    ? { stale: h.stale, hidden: h.hidden, archive: h.archive }
    : BALANCED;
}

const ROWS: { key: keyof Thresholds; label: Key; hint: Key }[] = [
  { key: 'stale', label: 'aging.row.stale', hint: 'aging.row.stale.hint' },
  { key: 'hidden', label: 'aging.row.hidden', hint: 'aging.row.hidden.hint' },
  { key: 'archive', label: 'aging.row.archive', hint: 'aging.row.archive.hint' },
];

// Aging (Designs/09): pace presets, custom thresholds in active hours, and what they mean.
export function Aging(props: TabProps) {
  const { settings, save } = props;
  const saved = readThresholds(settings?.thresholds);
  const [draft, setDraft] = useState<Thresholds>(saved);
  // A change from elsewhere (another window, a pace) replaces the draft.
  // biome-ignore lint/correctness/useExhaustiveDependencies: compare by value
  useEffect(() => setDraft(saved), [saved.stale, saved.hidden, saved.archive]);
  const pace: Pace = settings?.preset === 'custom' ? 'custom' : (paceOf(saved) as Pace);
  const problem = thresholdsProblem(draft);
  const dirty =
    draft.stale !== saved.stale || draft.hidden !== saved.hidden || draft.archive !== saved.archive;
  const choosePace = (p: Pace) => {
    if (p === 'custom') {
      if (problem) return; // shown inline; nothing to send
      save({ thresholds: draft }, t('aging.pace.saved', { pace: 'custom' }));
      return;
    }
    save({ preset: p }, t('aging.pace.saved', { pace: p }));
  };
  const total = Math.max(draft.archive * 1.15, 1);
  return (
    <>
      <TabHeader title={t('settings.tab.aging')}>{t('aging.subtitle')}</TabHeader>
      <div className="grid gap-4">
        <Group title={t('aging.pace')}>
          <div className="flex flex-wrap items-center justify-between gap-4">
            <p className="text-sm text-muted">{t('aging.pace.text')}</p>
            <SegmentedControl<Pace>
              label={t('aging.pace')}
              value={pace}
              onChange={choosePace}
              options={[
                { value: 'gentle', label: t('aging.pace.gentle') },
                { value: 'balanced', label: t('aging.pace.balanced') },
                { value: 'aggressive', label: t('aging.pace.aggressive') },
                { value: 'custom', label: t('aging.pace.custom') },
              ]}
            />
          </div>
        </Group>
        <Group title={t('aging.thresholds')} aside={t('aging.activeHours')}>
          <form
            className="grid gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              if (!problem && dirty) save({ thresholds: draft }, t('aging.saved'));
            }}
          >
            {ROWS.map((r) => (
              <div key={r.key} className="grid grid-cols-[1fr_auto_auto] items-center gap-3">
                <label htmlFor={`t-${r.key}`}>
                  <span className="block text-sm font-medium">{t(r.label)}</span>
                  <span className="block text-[13px] text-muted">{t(r.hint)}</span>
                </label>
                <span className="flex items-center gap-1.5">
                  <input
                    id={`t-${r.key}`}
                    type="number"
                    min={1}
                    max={720}
                    step="any"
                    value={Number.isFinite(draft[r.key]) ? draft[r.key] : ''}
                    onChange={(e) => setDraft({ ...draft, [r.key]: e.target.valueAsNumber })}
                    aria-describedby="t-help"
                    className="h-8 w-20 rounded-lg border border-line bg-surface-2 px-2 text-end font-mono text-sm"
                  />
                  <span className="text-xs text-muted">{t('aging.unit')}</span>
                </span>
                <span className="w-32 text-end text-xs text-muted">
                  {Number.isFinite(draft[r.key]) ? workTime(draft[r.key]) : ''}
                </span>
              </div>
            ))}
            <div aria-hidden className="mt-1 flex h-2 overflow-hidden rounded-full">
              <span className="bg-age-0" style={{ width: `${(draft.stale / total) * 100}%` }} />
              <span
                className="bg-age-1"
                style={{ width: `${((draft.hidden - draft.stale) / total) * 100}%` }}
              />
              <span
                className="bg-age-2"
                style={{ width: `${((draft.archive - draft.hidden) / total) * 100}%` }}
              />
              <span className="flex-1 bg-age-3" />
            </div>
            <p className="flex justify-between font-mono text-[11px] text-muted">
              <span>{t('aging.scale.fresh', { n: draft.stale })}</span>
              <span>{t('aging.scale.stale')}</span>
              <span>{t('aging.scale.hidden')}</span>
              <span>{t('aging.scale.archived', { n: draft.archive })}</span>
            </p>
            <div className="flex items-center justify-between gap-3">
              <p
                id="t-help"
                role={problem ? 'alert' : undefined}
                className={problem ? 'text-sm text-danger' : 'text-xs text-muted'}
              >
                {problem ?? t('aging.help')}
              </p>
              <span className="flex gap-2">
                {dirty && (
                  <Button size="sm" variant="ghost" onClick={() => setDraft(saved)}>
                    {t('aging.reset')}
                  </Button>
                )}
                <Button
                  size="sm"
                  variant="primary"
                  type="submit"
                  disabled={!dirty || problem !== null}
                >
                  {t('aging.save')}
                </Button>
              </span>
            </div>
          </form>
        </Group>
        <FeatureCard className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <p className="text-[11px] font-semibold tracking-wider text-feature-sub uppercase">
              {t('aging.preview')}
            </p>
            <p className="font-serif text-xl">{t('aging.preview.title')}</p>
            <p className="text-sm text-feature-sub">{t('aging.preview.text')}</p>
          </div>
          <Button
            variant="onFeature"
            onClick={() => api.showWindow('library', '/shadow-report').catch(props.fail)}
          >
            {t('aging.preview.open')}{' '}
            <ArrowRight className="rtl:-scale-x-100" size={14} aria-hidden />
          </Button>
        </FeatureCard>
      </div>
    </>
  );
}

export { PRESET_HOURS };
