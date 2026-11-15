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
import { matchHint, SCOPE_TEXT } from './model';
import { Group, Row, TabHeader, type TabProps } from './parts';

const RETENTION = [
  { value: '30', label: '30 days' },
  { value: '90', label: '90 days' },
  { value: '365', label: '1 year' },
  { value: '3650', label: '10 years' },
] as const;

// Privacy & data (Designs/09c, F9): how long the archive keeps things, and sites Decay
// never records ("never store" rules: adding one deletes what's stored for it).
export function Privacy(props: TabProps & { version: number; changed: () => void }) {
  const { settings, save, notify } = props;
  const rules = useLoad(() => api.listRules(), [props.version]);
  const excludes = (rules.data ?? []).filter((r) => r.action === 'exclude');
  const retention = String(settings?.retention_days ?? 90);
  return (
    <>
      <TabHeader title="Privacy & data">
        Everything stays on this device, encrypted. No account, no tracking.
      </TabHeader>
      <div className="grid gap-4">
        <Group title="Archive">
          <Row
            label="Keep archived items for"
            description="Older items are removed for good, daily. Restored items are kept with their tab."
          >
            <SegmentedControl
              label="Keep archived items for"
              value={RETENTION.some((r) => r.value === retention) ? retention : '90'}
              onChange={(v) => save({ retention_days: Number(v) }, 'Saved')}
              options={[...RETENTION]}
            />
          </Row>
        </Group>
        <Group
          title="Never recorded"
          aside={excludes.length === 1 ? '1 site' : `${excludes.length} sites`}
        >
          <p className="pb-3 text-[13px] text-muted">
            Decay never records these pages, even in its history. Private windows are never recorded
            either.
          </p>
          {rules.error && <Banner tone="danger">{rules.error}</Banner>}
          <ul className="grid gap-1 pb-3">
            {excludes.map((r) => (
              <ExcludeRow
                key={r.id}
                rule={r}
                onChanged={(t) => {
                  notify(t);
                  props.changed();
                }}
              />
            ))}
            {excludes.length === 0 && !rules.loading && (
              <li className="text-sm text-muted">No sites yet.</li>
            )}
          </ul>
          <AddExclude
            onAdded={(t) => {
              notify(t);
              props.changed();
            }}
          />
        </Group>
        <Transfer notify={notify} changed={props.changed} />
        <Diagnostics crashReports={settings?.crash_reports === true} save={save} notify={notify} />
        <p className="flex items-center gap-2 text-xs text-muted">
          <Lock size={12} aria-hidden /> Erasing all data comes in a later version.
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
    ? "Couldn't check whether this build can send crash reports."
    : available || info.loading
      ? 'Helps fix bugs without sending URLs, titles, or browsing habits.'
      : "This build can't send crash reports.";
  const run = (action: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setError('');
    action()
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setBusy(false));
  };
  return (
    <Group title="Diagnostics">
      <Switch
        className="pb-3"
        label="Share anonymous crash reports"
        description={description}
        checked={props.crashReports && (available || info.loading)}
        disabled={!available}
        onChange={(on) =>
          props.save({ crash_reports: on }, on ? 'Crash reports on' : 'Crash reports off')
        }
      />
      <Row
        label="Log files"
        description={
          info.data
            ? `Stored on this device in ${info.data.logsDir}, 7 days kept. No addresses or titles.`
            : 'Stored on this device for troubleshooting. No addresses or titles.'
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
            Open logs folder
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
                    props.notify(`Diagnostics saved to ${name}`);
                  }
                }),
              )
            }
          >
            {busy ? 'Exporting…' : 'Export diagnostics…'}
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

const EXPORTS: { format: ExportFormat; label: string }[] = [
  { format: 'json', label: 'Export JSON' },
  { format: 'html', label: 'Export bookmarks (HTML)' },
  { format: 'csv', label: 'Export CSV' },
];

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
    <Group title="Export & import">
      <div className="grid gap-3">
        <p className="text-[13px] text-muted">
          Save your archive as a file, or bring in one from another Mac or PC. JSON keeps
          everything, including rules; bookmarks open in any browser. Importing keeps what's here,
          saves a backup copy first and never adds sites you chose not to record. Imported bookmarks
          are kept like archived tabs.
        </p>
        <div className="flex flex-wrap gap-2" aria-busy={busy !== ''}>
          {EXPORTS.map((x) => (
            <Button
              key={x.format}
              size="sm"
              variant="secondary"
              aria-disabled={busy !== '' || waiting !== null}
              icon={<Download size={14} aria-hidden />}
              onClick={() =>
                !waiting &&
                act('export', () =>
                  api.exportArchive(x.format).then((r) => {
                    if (r) props.notify(exportSummary(r));
                  }),
                )
              }
            >
              {x.label}
            </Button>
          ))}
          <Button
            size="sm"
            variant="secondary"
            aria-disabled={busy !== '' || waiting !== null}
            icon={<Upload size={14} aria-hidden />}
            onClick={() => !waiting && act('import', () => api.importArchive().then(finish))}
          >
            Import…
          </Button>
        </div>
        <p role="status" className="text-sm text-muted empty:hidden">
          {busy === 'export' ? 'Exporting…' : busy === 'import' ? 'Importing…' : ''}
        </p>
        {waiting && (
          <Banner
            tone="warning"
            action={
              <span className="flex gap-2">
                <Button size="sm" variant="danger" autoFocus onClick={() => answer(true)}>
                  {`Delete ${waiting.purge} and import`}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => answer(false)}>
                  Cancel
                </Button>
              </span>
            }
          >
            {`${waiting.fileName} has never-record rules that cover ${waiting.purge} stored ${
              waiting.purge === 1 ? 'item' : 'items'
            }. Importing it deletes ${waiting.purge === 1 ? 'it' : 'them'} for good.`}
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
      {!r.enabled && <span className="text-xs text-muted">Off: still recorded</span>}
      <span className="text-xs text-muted">{SCOPE_TEXT[r.scope]}</span>
      {error && (
        <span role="alert" className="text-xs text-danger">
          {error}
        </span>
      )}
      <IconButton
        label={`Record ${r.match} again`}
        icon={<Trash2 size={14} />}
        onClick={() =>
          api
            .deleteRule(r.id)
            .then(() => props.onChanged(`Decay records ${r.match} again`))
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
            ? `Decay no longer records ${r.rule.match}; ${r.purged} stored items deleted`
            : `Decay no longer records ${r.rule.match}`,
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
          label="Never record a"
          value={scope}
          onChange={(s) => {
            setScope(s);
            setConfirm(null);
            setError('');
          }}
          options={[
            { value: 'domain', label: 'Site' },
            { value: 'urlGlob', label: 'Page address' },
            { value: 'titleRegex', label: 'Title' },
          ]}
        />
        <label className="sr-only" htmlFor="exclude-match">
          Never record
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
          Add
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
                {confirm > 0 ? `Delete ${confirm} and add` : 'Add'}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setConfirm(null)}>
                Cancel
              </Button>
            </span>
          }
        >
          {confirm > 0
            ? `${confirm} stored ${confirm === 1 ? 'item matches' : 'items match'} (open tabs and archived ones). They'll be deleted for good.`
            : 'Nothing stored matches yet.'}
        </Banner>
      )}
    </form>
  );
}
