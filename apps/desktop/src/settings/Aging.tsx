import { Button, FeatureCard, SegmentedControl } from '@decay/ui';
import { ArrowRight } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, type Thresholds } from '../api';
import { BALANCED, PRESET_HOURS, paceOf, thresholdsProblem, workTime } from './model';
import { Group, TabHeader, type TabProps } from './parts';

type Pace = 'gentle' | 'balanced' | 'aggressive' | 'custom';

function readThresholds(v: unknown): Thresholds {
  const t = v as Partial<Thresholds> | null;
  return t &&
    typeof t.stale === 'number' &&
    typeof t.hidden === 'number' &&
    typeof t.archive === 'number'
    ? { stale: t.stale, hidden: t.hidden, archive: t.archive }
    : BALANCED;
}

const ROWS: { key: keyof Thresholds; label: string; hint: string }[] = [
  { key: 'stale', label: 'Mark as stale after', hint: 'Dimmed, and counted in the menu bar badge' },
  { key: 'hidden', label: 'Hide windows after', hint: 'App windows tucked away (minimised)' },
  { key: 'archive', label: 'Archive tabs after', hint: 'Saved safely to the Library, then closed' },
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
      save({ thresholds: draft }, 'Custom pace');
      return;
    }
    save({ preset: p }, `${p[0]?.toUpperCase()}${p.slice(1)} pace`);
  };
  const total = Math.max(draft.archive * 1.15, 1);
  return (
    <>
      <TabHeader title="Aging">
        How long things can sit unused before Decay steps in. Time counts only while you're active.
      </TabHeader>
      <div className="grid gap-4">
        <Group title="Pace">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <p className="text-sm text-muted">A preset for archiving tabs and windows</p>
            <SegmentedControl<Pace>
              label="Pace"
              value={pace}
              onChange={choosePace}
              options={[
                { value: 'gentle', label: 'Gentle' },
                { value: 'balanced', label: 'Balanced' },
                { value: 'aggressive', label: 'Aggressive' },
                { value: 'custom', label: 'Custom' },
              ]}
            />
          </div>
        </Group>
        <Group title="Thresholds" aside="Active hours">
          <form
            className="grid gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              if (!problem && dirty) save({ thresholds: draft }, 'Thresholds saved');
            }}
          >
            {ROWS.map((r) => (
              <div key={r.key} className="grid grid-cols-[1fr_auto_auto] items-center gap-3">
                <label htmlFor={`t-${r.key}`}>
                  <span className="block text-sm font-medium">{r.label}</span>
                  <span className="block text-[13px] text-muted">{r.hint}</span>
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
                    className="h-8 w-20 rounded-lg border border-line bg-surface-2 px-2 text-right font-mono text-sm"
                  />
                  <span className="text-xs text-muted">active h</span>
                </span>
                <span className="w-32 text-right text-xs text-muted">
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
              <span>Fresh 0–{draft.stale} h</span>
              <span>Stale</span>
              <span>Hidden</span>
              <span>Archived {draft.archive} h+</span>
            </p>
            <div className="flex items-center justify-between gap-3">
              <p
                id="t-help"
                role={problem ? 'alert' : undefined}
                className={problem ? 'text-sm text-danger' : 'text-xs text-muted'}
              >
                {problem ??
                  'Saving sets the pace to Custom. Rules can change timing for single sites.'}
              </p>
              <span className="flex gap-2">
                {dirty && (
                  <Button size="sm" variant="ghost" onClick={() => setDraft(saved)}>
                    Reset
                  </Button>
                )}
                <Button
                  size="sm"
                  variant="primary"
                  type="submit"
                  disabled={!dirty || problem !== null}
                >
                  Save thresholds
                </Button>
              </span>
            </div>
          </form>
        </Group>
        <FeatureCard className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <p className="text-[11px] font-semibold tracking-wider text-feature-sub uppercase">
              Preview
            </p>
            <p className="font-serif text-xl">See what these settings would do</p>
            <p className="text-sm text-feature-sub">
              The Shadow review lists what Decay would archive and why.
            </p>
          </div>
          <Button
            variant="onFeature"
            onClick={() => api.showWindow('library', '/shadow-report').catch(props.fail)}
          >
            Open shadow review <ArrowRight size={14} aria-hidden />
          </Button>
        </FeatureCard>
      </div>
    </>
  );
}

export { PRESET_HOURS };
