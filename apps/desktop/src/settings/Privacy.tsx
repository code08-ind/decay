import { Banner, Button, IconButton, SegmentedControl, Switch } from '@decay/ui';
import { Download, EyeOff, Lock, Trash2, Upload } from 'lucide-react';
import { useState } from 'react';
import {
  api,
  type ExportFormat,
  errorMessage,
  exportSummary,
  type ImportOutcome,
  importSummary,
  type Rule,
  type RuleScope,
} from '../api';
import { useLoad } from '../library/parts';
import { t } from '../strings';
import { matchHint, scopeText } from './model';
import { Group, Row, TabHeader, type TabProps } from './parts';

const RETENTION = ['30', '90', '365', '3650'] as const;

// Privacy & data (Designs/09c, F9): how long the archive keeps things, and sites Decay
// never records ("never store" rules: adding one deletes what's stored for it).
export function Privacy(props: TabProps & { version: number; changed: () => void }) {
  const { settings, save, notify } = props;
  const rules = useLoad(() => api.listRules(), [props.version]);
  const excludes = (rules.data ?? []).filter((r) => r.action === 'exclude');
  const retention = String(settings?.retention_days ?? 90);
  return (
    <>
      <TabHeader title={t('settings.tab.privacy')}>{t('privacy.subtitle')}</TabHeader>
      <div className="grid gap-4">
        <Group title={t('privacy.archive')}>
          <Row label={t('privacy.keep')} description={t('privacy.keep.text')}>
            <SegmentedControl
              label={t('privacy.keep')}
              value={RETENTION.some((r) => r === retention) ? retention : '90'}
              onChange={(v) => save({ retention_days: Number(v) }, t('common.saved'))}
              options={RETENTION.map((value) => ({
                value,
                label: t(`privacy.retention.${value}`),
              }))}
            />
          </Row>
        </Group>
        <Group title={t('privacy.never')} aside={t('privacy.never.count', { n: excludes.length })}>
          <p className="pb-3 text-[13px] text-muted">{t('privacy.never.text')}</p>
          {rules.error && <Banner tone="danger">{rules.error}</Banner>}
          <ul className="grid gap-1 pb-3">
            {excludes.map((r) => (
              <ExcludeRow
                key={r.id}
                rule={r}
                onChanged={(text) => {
                  notify(text);
                  props.changed();
                }}
              />
            ))}
            {excludes.length === 0 && !rules.loading && (
              <li className="text-sm text-muted">{t('privacy.never.empty')}</li>
            )}
          </ul>
          <AddExclude
            onAdded={(text) => {
              notify(text);
              props.changed();
            }}
          />
        </Group>
        <Transfer notify={notify} changed={props.changed} />
        <Diagnostics crashReports={settings?.crash_reports === true} save={save} notify={notify} />
        <p className="flex items-center gap-2 text-xs text-muted">
          <Lock size={12} aria-hidden /> {t('privacy.eraseLater')}
        </p>
      </div>
    </>
  );
}

/**
 * Diagnostics (F29, Designs/09c): opt-in crash reports, the logs folder, and a
 * diagnostics zip (no addresses, titles or sites in either).
 */
function Diagnostics(props: {
  crashReports: boolean;
  save: TabProps['save'];
  notify: (text: string) => void;
}) {
  const info = useLoad(() => api.getDiagnosticsInfo(), []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const available = info.data?.crashReportsAvailable === true;
  const description = info.error
    ? t('diag.cantCheck')
    : available || info.loading
      ? t('diag.helps')
      : t('diag.unavailable');
  const run = (action: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setError('');
    action()
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setBusy(false));
  };
  return (
    <Group title={t('diag.title')}>
      <Switch
        className="pb-3"
        label={t('diag.crash')}
        description={description}
        checked={props.crashReports && (available || info.loading)}
        disabled={!available}
        onChange={(on) =>
          props.save({ crash_reports: on }, on ? t('diag.crashOn') : t('diag.crashOff'))
        }
      />
      <Row
        label={t('diag.logs')}
        description={
          info.data ? t('diag.logs.where', { dir: info.data.logsDir }) : t('diag.logs.text')
        }
      >
        <span className="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              setError('');
              api.openLogsFolder().catch((e) => setError(errorMessage(e)));
            }}
          >
            {t('diag.openLogs')}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            aria-disabled={busy}
            onClick={() =>
              run(() =>
                api.exportDiagnostics().then((path) => {
                  if (path) {
                    const name = path.split(/[\\/]/).pop() ?? path;
                    props.notify(t('diag.saved', { name }));
                  }
                }),
              )
            }
          >
            {busy ? t('diag.exporting') : t('diag.export')}
          </Button>
        </span>
      </Row>
      {error && (
        <p role="alert" className="pt-2 text-sm text-danger">
          {error}
        </p>
      )}
    </Group>
  );
}

const EXPORTS: ExportFormat[] = ['json', 'html', 'csv'];

type Waiting = { token: number; purge: number; fileName: string };

/**
 * Export & import (F20): the files are picked in the system's own dialogs. A file whose
 * "never record" rules would delete stored items waits for a confirmation, as adding
 * such a rule by hand does. Buttons stay focusable while busy (`aria-disabled`), so
 * keyboard focus isn't lost.
 */
function Transfer(props: { notify: (text: string) => void; changed: () => void }) {
  const [busy, setBusy] = useState<'' | 'export' | 'import'>('');
  const [error, setError] = useState('');
  const [waiting, setWaiting] = useState<Waiting | null>(null);
  const finish = (outcome: ImportOutcome | null) => {
    if (!outcome) return;
    if ('confirm' in outcome) {
      setWaiting(outcome.confirm);
      return;
    }
    props.notify(importSummary(outcome.done));
    props.changed();
  };
  const act = (what: 'export' | 'import', action: () => Promise<void>) => {
    if (busy) return;
    setBusy(what);
    setError('');
    action()
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setBusy(''));
  };
  const answer = (proceed: boolean) => {
    if (!waiting) return;
    const { token } = waiting;
    setWaiting(null);
    act('import', () => api.confirmImport(token, proceed).then(proceed ? finish : () => {}));
  };
  return (
    <Group title={t('transfer.title')}>
      <div className="grid gap-3">
        <p className="text-[13px] text-muted">{t('transfer.text')}</p>
        <div className="flex flex-wrap gap-2" aria-busy={busy !== ''}>
          {EXPORTS.map((format) => (
            <Button
              key={format}
              size="sm"
              variant="secondary"
              aria-disabled={busy !== '' || waiting !== null}
              icon={<Download size={14} aria-hidden />}
              onClick={() =>
                !waiting &&
                act('export', () =>
                  api.exportArchive(format).then((r) => {
                    if (r) props.notify(exportSummary(r));
                  }),
                )
              }
            >
              {t(`transfer.${format}`)}
            </Button>
          ))}
          <Button
            size="sm"
            variant="secondary"
            aria-disabled={busy !== '' || waiting !== null}
            icon={<Upload size={14} aria-hidden />}
            onClick={() => !waiting && act('import', () => api.importArchive().then(finish))}
          >
            {t('transfer.import')}
          </Button>
        </div>
        <p role="status" className="text-sm text-muted empty:hidden">
          {busy === 'export'
            ? t('transfer.exporting')
            : busy === 'import'
              ? t('transfer.importing')
              : ''}
        </p>
        {waiting && (
          <Banner
            tone="warning"
            action={
              <span className="flex gap-2">
                <Button size="sm" variant="danger" autoFocus onClick={() => answer(true)}>
                  {t('transfer.confirm', { n: waiting.purge })}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => answer(false)}>
                  {t('common.cancel')}
                </Button>
              </span>
            }
          >
            {t('transfer.confirm.text', { file: waiting.fileName, n: waiting.purge })}
          </Banner>
        )}
        {error && (
          <p role="alert" className="text-sm text-danger">
            {error}
          </p>
        )}
      </div>
    </Group>
  );
}

function ExcludeRow(props: { rule: Rule; onChanged: (text: string) => void }) {
  const [error, setError] = useState('');
  const r = props.rule;
  return (
    <li className="flex items-center gap-3 rounded-lg bg-surface-2 px-3 py-2">
      <EyeOff size={14} aria-hidden className="text-muted" />
      <span className="min-w-0 flex-1 truncate font-mono text-[13px]">{r.match}</span>
      {!r.enabled && <span className="text-xs text-muted">{t('exclude.off')}</span>}
      <span className="text-xs text-muted">{scopeText(r.scope)}</span>
      {error && (
        <span role="alert" className="text-xs text-danger">
          {error}
        </span>
      )}
      <IconButton
        label={t('exclude.remove', { match: r.match })}
        icon={<Trash2 size={14} />}
        onClick={() =>
          api
            .deleteRule(r.id)
            .then(() => props.onChanged(t('exclude.removed', { match: r.match })))
            .catch((e) => setError(errorMessage(e)))
        }
      />
    </li>
  );
}

/** Adds a "never store" rule after showing how much stored data it deletes. */
function AddExclude(props: { onAdded: (text: string) => void }) {
  const [scope, setScope] = useState<RuleScope>('domain');
  const [match, setMatch] = useState('');
  const [confirm, setConfirm] = useState<number | null>(null);
  const [error, setError] = useState('');
  const input = { scope, match, action: 'exclude' as const };
  const check = () => {
    setError('');
    api
      .previewExclude(input)
      .then((r) => setConfirm(r.count))
      .catch((e) => setError(errorMessage(e)));
  };
  const add = () =>
    api
      .createRule(input)
      .then((r) => {
        setMatch('');
        setConfirm(null);
        props.onAdded(
          r.purged > 0
            ? t('exclude.addedPurged', { match: r.rule.match, n: r.purged })
            : t('exclude.added', { match: r.rule.match }),
        );
      })
      .catch((e) => {
        setConfirm(null);
        setError(errorMessage(e));
      });
  return (
    <form
      className="grid gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (match.trim()) check();
      }}
    >
      <div className="flex flex-wrap items-center gap-2">
        <SegmentedControl<RuleScope>
          label={t('exclude.scope')}
          value={scope}
          onChange={(s) => {
            setScope(s);
            setConfirm(null);
            setError('');
          }}
          options={(['domain', 'urlGlob', 'titleRegex'] as const).map((value) => ({
            value,
            label: scopeText(value),
          }))}
        />
        <label className="sr-only" htmlFor="exclude-match">
          {t('exclude.match')}
        </label>
        <input
          id="exclude-match"
          value={match}
          placeholder={matchHint(scope)}
          onChange={(e) => {
            setMatch(e.target.value);
            setConfirm(null);
          }}
          className="h-8 min-w-48 flex-1 rounded-lg border border-line bg-surface-2 px-3 font-mono text-[13px]"
        />
        <Button type="submit" size="sm" disabled={!match.trim()}>
          {t('exclude.add')}
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      {confirm !== null && (
        <Banner
          tone={confirm > 0 ? 'warning' : 'info'}
          action={
            <span className="flex gap-2">
              <Button
                size="sm"
                variant={confirm > 0 ? 'danger' : 'primary'}
                autoFocus
                onClick={add}
              >
                {confirm > 0 ? t('exclude.confirm', { n: confirm }) : t('exclude.add')}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setConfirm(null)}>
                {t('common.cancel')}
              </Button>
            </span>
          }
        >
          {confirm > 0 ? t('exclude.matches', { n: confirm }) : t('exclude.nothing')}
        </Banner>
      )}
    </form>
  );
}
