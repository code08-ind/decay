import { Button, Kbd, Shortcut } from '@decay/ui';
import { type KeyboardEvent, useState } from 'react';
import { api, errorMessage } from '../api';
import { type Key, t } from '../strings';
import { acceleratorFrom } from './model';
import { Group, Row, TabHeader, type TabProps } from './parts';

const IN_APP: [Key, string][] = [
  ['shortcuts.inApp.restore', 'Enter'],
  ['shortcuts.inApp.group', 'CmdOrCtrl+Enter'],
  ['shortcuts.inApp.newWindow', 'Alt+Enter'],
  ['shortcuts.inApp.keep', 'CmdOrCtrl+P'],
  ['shortcuts.inApp.details', 'CmdOrCtrl+I'],
  ['shortcuts.inApp.close', 'Escape'],
  ['shortcuts.inApp.list', 'CmdOrCtrl+/'],
];

// Shortcuts (Designs/09d): record the global search shortcut; the in-app keys.
export function Shortcuts(props: TabProps) {
  const { settings, notify } = props;
  const current = typeof settings?.hotkey === 'string' ? settings.hotkey : '';
  const [recording, setRecording] = useState(false);
  const [candidate, setCandidate] = useState<string | null>(null);
  const [hint, setHint] = useState('');
  const onKey = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (!recording) return;
    if (e.key === 'Tab') return; // keep keyboard navigation
    e.preventDefault();
    if (e.key === 'Escape' && !e.altKey && !e.ctrlKey && !e.metaKey) {
      setRecording(false);
      setCandidate(null);
      setHint('');
      return;
    }
    const acc = acceleratorFrom(e);
    if (acc) {
      setCandidate(acc);
      setHint('');
    } else if (!['Alt', 'Control', 'Meta', 'Shift'].includes(e.key)) {
      setHint(t('shortcuts.needModifier'));
    }
  };
  const use = () => {
    if (!candidate) return;
    api
      .setHotkey(candidate)
      .then(() => {
        notify(t('shortcuts.changed'));
        setRecording(false);
        setCandidate(null);
      })
      .catch((e) => {
        // One message, next to the recorder (not the top banner too).
        setHint(t('shortcuts.tryAnother', { error: errorMessage(e) || t('shortcuts.taken') }));
      });
  };
  return (
    <>
      <TabHeader title={t('settings.tab.shortcuts')}>{t('shortcuts.subtitle')}</TabHeader>
      <div className="grid gap-4">
        <Group title={t('shortcuts.global')}>
          <Row label={t('shortcuts.search')} description={t('shortcuts.search.text')}>
            {current && <Shortcut accelerator={current} />}
          </Row>
          <div className="grid gap-2 pt-3">
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                aria-pressed={recording}
                aria-describedby="hotkey-hint"
                onClick={() => {
                  setRecording(!recording);
                  setCandidate(null);
                  setHint('');
                }}
                onKeyDown={onKey}
                className="h-9 min-w-56 rounded-lg border border-line-strong bg-surface-2 px-3 text-start text-sm"
              >
                {recording ? (
                  candidate ? (
                    <Shortcut accelerator={candidate} />
                  ) : (
                    t('shortcuts.press')
                  )
                ) : (
                  t('shortcuts.record')
                )}
              </button>
              {recording && candidate && (
                <Button size="sm" variant="primary" onClick={use}>
                  {t('shortcuts.use')}
                </Button>
              )}
            </div>
            <p id="hotkey-hint" role={hint ? 'alert' : undefined} className="text-xs text-muted">
              {hint || (recording ? t('shortcuts.escCancels') : t('shortcuts.howTo'))}
            </p>
          </div>
        </Group>
        <Group title={t('shortcuts.inApp')}>
          {IN_APP.map(([what, keys]) => (
            <Row key={what} label={t(what)}>
              {keys === 'Enter' || keys === 'Escape' ? (
                <Kbd>{keys === 'Enter' ? '↵' : 'Esc' /* i18n-ignore: a key cap */}</Kbd>
              ) : (
                <Shortcut accelerator={keys} />
              )}
            </Row>
          ))}
        </Group>
      </div>
    </>
  );
}
