import { Banner, Button, Chip, cx, IconButton, SegmentedControl, Switch } from '@decay/ui';
import { Filter, Info, Pencil, Plus, ShieldCheck, Trash2, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import {
  api,
  errorMessage,
  type Rule,
  type RuleScope,
  type RuleTestResult,
  type Thresholds,
} from '../api';
import { useLoad } from '../library/parts';
import { displayUrl } from '../panel/results';
import { type Key, t } from '../strings';
import {
  actionChip,
  BALANCED,
  type Intent,
  intentOf,
  matchHint,
  ruleInput,
  ruleSentence,
  scopeText,
  sliderFactor,
} from './model';
import { Group, TabHeader, type TabProps } from './parts';

const BUILT_IN: Key[] = [
  'protection.builtin.pinned',
  'protection.builtin.front',
  'protection.builtin.forms',
  'protection.builtin.restored',
  'protection.builtin.exact',
];

type Editing = { rule: Rule | null } | null;

// Protection (Designs/08, 09b; F8): built-in protections, and the rules list with an
// editor that tests a rule against open tabs while it's written. "Never store" rules
// live in Privacy & data.
export function Protection(props: TabProps & { version: number; changed: () => void }) {
  const { settings, notify } = props;
  const global = (settings?.thresholds as Thresholds | undefined) ?? BALANCED;
  const rules = useLoad(() => api.listRules(), [props.version]);
  const [editing, setEditing] = useState<Editing>(null);
  const [error, setError] = useState('');
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const shown = (rules.data ?? []).filter((r) => r.action !== 'exclude');
  const run = (p: Promise<unknown>, done: string) =>
    p
      .then(() => {
        setError('');
        notify(done);
        props.changed();
      })
      .catch((e) => setError(errorMessage(e)));
  const toggle = (r: Rule, on: boolean) =>
    run(
      api.updateRule(r.id, {
        scope: r.scope,
        match: r.match,
        action: r.action,
        thresholdHours: r.thresholdHours,
        enabled: on,
      }),
      on ? t('protection.ruleOn') : t('protection.ruleOff'),
    );
  return (
    <>
      <TabHeader title={t('settings.tab.protection')}>{t('protection.subtitle')}</TabHeader>
      <div className="grid gap-4">
        <Group title={t('protection.always')}>
          <ul className="grid gap-1.5 text-sm">
            {BUILT_IN.map((b) => (
              <li key={b} className="flex items-center gap-2">
                <ShieldCheck size={14} aria-hidden className="shrink-0 text-accent-text" /> {t(b)}
              </li>
            ))}
          </ul>
        </Group>
        {error && <Banner tone="danger">{error}</Banner>}
        {rules.error && <Banner tone="danger">{rules.error}</Banner>}
        <section
          aria-labelledby="rules-h"
          className="rounded-card border border-line bg-surface shadow-sm"
        >
          <div className="flex items-center justify-between gap-3 border-b border-line px-5 py-3">
            <h2
              id="rules-h"
              className="text-[11px] font-semibold tracking-wider text-muted uppercase"
            >
              {t('protection.rules')}{' '}
              <span className="ms-1 rounded bg-surface-2 px-1.5">{shown.length}</span>
            </h2>
            <Button
              size="sm"
              variant="primary"
              icon={<Plus size={14} aria-hidden />}
              onClick={() => setEditing({ rule: null })}
            >
              {t('protection.new')}
            </Button>
          </div>
          {shown.length === 0 && !rules.loading ? (
            <p className="px-5 py-6 text-sm text-muted">{t('protection.empty')}</p>
          ) : (
            <ul className="divide-y divide-line">
              {shown.map((r) => (
                <li key={r.id} className="flex flex-wrap items-center gap-3 px-5 py-3">
                  <Switch
                    label={t('protection.toggle', {
                      action: actionChip(r, global),
                      match: r.match,
                    })}
                    className="[&_label]:sr-only"
                    checked={r.enabled}
                    onChange={(on) => toggle(r, on)}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-mono text-[13px]">
                      {r.scope === 'titleRegex'
                        ? t('protection.titleMatch', { match: r.match })
                        : r.match}
                    </span>
                    <span className="block text-[13px] text-muted">{ruleSentence(r)}</span>
                  </span>
                  <Chip>{scopeText(r.scope)}</Chip>
                  <Chip tone={intentOf(r, global) === 'sooner' ? 'warning' : 'accent'}>
                    {actionChip(r, global)}
                  </Chip>
                  {SCOPES.some((x) => x.value === r.scope) && r.action !== 'closeAllowed' && (
                    <IconButton
                      label={t('protection.edit', { match: r.match })}
                      icon={<Pencil size={14} />}
                      onClick={() => setEditing({ rule: r })}
                    />
                  )}
                  {confirmDelete === r.id ? (
                    <span className="flex gap-1">
                      <Button
                        size="sm"
                        variant="danger"
                        autoFocus
                        onClick={() =>
                          run(api.deleteRule(r.id), t('protection.deleted')).then(() =>
                            setConfirmDelete(null),
                          )
                        }
                      >
                        {t('protection.delete')}
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => setConfirmDelete(null)}>
                        {t('common.cancel')}
                      </Button>
                    </span>
                  ) : (
                    <IconButton
                      label={t('protection.deleteFor', { match: r.match })}
                      icon={<Trash2 size={14} />}
                      onClick={() => setConfirmDelete(r.id)}
                    />
                  )}
                </li>
              ))}
            </ul>
          )}
          <p className="flex items-center gap-2 border-t border-line px-5 py-3 text-xs text-muted">
            <Info size={14} aria-hidden /> {t('protection.precedence')}
          </p>
        </section>
        {editing && (
          <RuleEditor
            key={editing.rule?.id ?? 'new'}
            rule={editing.rule}
            global={global}
            onClose={() => setEditing(null)}
            onSaved={(text) => {
              setEditing(null);
              notify(text);
              props.changed();
            }}
          />
        )}
      </div>
    </>
  );
}

const SCOPES: { value: RuleScope }[] = [
  { value: 'domain' },
  { value: 'urlGlob' },
  { value: 'titleRegex' },
];

/** The add / edit panel: when, matches, do, how much, and a live test. */
export function RuleEditor(props: {
  rule: Rule | null;
  global: Thresholds;
  onClose: () => void;
  onSaved: (text: string) => void;
}) {
  const r = props.rule;
  const [scope, setScope] = useState<RuleScope>(
    r && SCOPES.some((s) => s.value === r.scope) ? r.scope : 'domain',
  );
  const [match, setMatch] = useState(r?.match ?? '');
  const [intent, setIntent] = useState<Exclude<Intent, 'exclude'>>(
    r ? (intentOf(r, props.global) as Exclude<Intent, 'exclude'>) : 'protect',
  );
  const startIntent = r ? intentOf(r, props.global) : 'protect';
  const [factor, setFactor] = useState(() =>
    r?.action === 'customThreshold' ? sliderFactor(r, props.global) : 2,
  );
  // A timing rule keeps its own hours until the slider or "Do" changes.
  const [factorMoved, setFactorMoved] = useState(false);
  const [test, setTest] = useState<RuleTestResult | null>(null);
  const [problem, setProblem] = useState('');
  const [saving, setSaving] = useState(false);
  const scaledInput = ruleInput(scope, match, intent, factor, props.global, r?.enabled ?? true);
  const keepHours =
    r?.action === 'customThreshold' && r.thresholdHours && !factorMoved && intent === startIntent;
  const input = keepHours ? { ...scaledInput, thresholdHours: r.thresholdHours } : scaledInput;
  const first = useRef<HTMLInputElement>(null);
  useEffect(() => first.current?.focus(), []);
  // Test while typing (a short pause after the last key).
  const key = JSON.stringify(input);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands for the input
  useEffect(() => {
    if (!match.trim()) {
      setTest(null);
      setProblem('');
      return;
    }
    let live = true;
    const timer = setTimeout(() => {
      api
        .testRule(input)
        .then((res) => {
          if (!live) return;
          setTest(res);
          setProblem('');
        })
        .catch((e) => {
          if (!live) return;
          setTest(null);
          setProblem(errorMessage(e));
        });
    }, 250);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [key]);
  const save = () => {
    setSaving(true);
    const done = r ? api.updateRule(r.id, input) : api.createRule(input);
    done
      .then(() => props.onSaved(r ? t('protection.saved') : t('protection.added')))
      .catch((e) => setProblem(errorMessage(e)))
      .finally(() => setSaving(false));
  };
  return (
    <section
      aria-labelledby="editor-h"
      className="rounded-card border border-line bg-surface p-5 shadow-sm"
      onKeyDown={(e) => {
        if (e.key === 'Escape') props.onClose();
      }}
    >
      <div className="mb-4 flex items-center justify-between">
        <h2 id="editor-h" className="font-serif text-2xl">
          {r ? t('editor.edit') : t('editor.new')}
        </h2>
        <IconButton
          label={t('editor.close')}
          icon={<X size={16} />}
          outlined
          onClick={props.onClose}
        />
      </div>
      <form
        className="grid gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          if (match.trim() && !problem) save();
        }}
      >
        <div className="grid gap-1.5">
          <span className="text-sm">{t('editor.when')}</span>
          <SegmentedControl<RuleScope>
            label={t('editor.when')}
            value={scope}
            onChange={setScope}
            options={SCOPES.map((s) => ({ value: s.value, label: scopeText(s.value) }))}
          />
        </div>
        <label className="grid gap-1.5">
          <span className="text-sm">{t('editor.matches')}</span>
          <input
            ref={first}
            value={match}
            onChange={(e) => setMatch(e.target.value)}
            placeholder={matchHint(scope)}
            aria-invalid={problem !== ''}
            aria-describedby={problem ? 'rule-problem' : undefined}
            className="h-9 rounded-lg border border-line bg-surface-2 px-3 font-mono text-sm"
          />
        </label>
        <div className="grid gap-1.5">
          <span className="text-sm">{t('editor.do')}</span>
          <SegmentedControl<Exclude<Intent, 'exclude'>>
            label={t('editor.do')}
            value={intent}
            onChange={setIntent}
            options={[
              { value: 'protect', label: t('editor.do.protect') },
              { value: 'longer', label: t('editor.do.longer') },
              { value: 'sooner', label: t('editor.do.sooner') },
            ]}
          />
        </div>
        {intent !== 'protect' && (
          <label className="grid gap-1.5">
            <span className="flex justify-between text-sm">
              <span>{intent === 'longer' ? t('editor.howLonger') : t('editor.howSooner')}</span>
              <Chip tone={intent === 'longer' ? 'accent' : 'warning'}>
                {t('editor.factor', {
                  intent,
                  factor,
                  hours: input.thresholdHours?.archive ?? '',
                })}
              </Chip>
            </span>
            <input
              type="range"
              min={1.25}
              max={5}
              step={0.25}
              value={factor}
              onChange={(e) => {
                setFactor(e.target.valueAsNumber);
                setFactorMoved(true);
              }}
              className="accent-[var(--accent)]"
            />
          </label>
        )}
        <p
          id="rule-problem"
          role={problem ? 'alert' : undefined}
          className={cx('text-sm', problem ? 'text-danger' : 'sr-only')}
        >
          {problem}
        </p>
        {test && (
          <div className="rounded-xl bg-feature p-3 text-feature-ink">
            <p className="mb-2 flex items-center gap-2 text-sm">
              <Filter size={14} aria-hidden />
              {test.matchesArchived > 0
                ? t('editor.testArchived', { n: test.matchesLive, archived: test.matchesArchived })
                : t('editor.test', { n: test.matchesLive })}
            </p>
            <ul className="grid gap-1">
              {test.sample.map((i) => (
                <li
                  key={i.id}
                  className="truncate rounded-lg bg-feature-tile px-2.5 py-1.5 text-[13px]"
                >
                  {i.title || displayUrl(i.url)}
                </li>
              ))}
            </ul>
          </div>
        )}
        <div className="flex justify-end gap-2">
          <Button onClick={props.onClose}>{t('common.cancel')}</Button>
          <Button
            type="submit"
            variant="primary"
            loading={saving}
            disabled={!match.trim() || problem !== ''}
          >
            {r ? t('editor.save') : t('editor.add')}
          </Button>
        </div>
      </form>
    </section>
  );
}
