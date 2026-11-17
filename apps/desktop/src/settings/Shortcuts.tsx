import { Button, Kbd, Shortcut } from '@decay/ui';
import { type KeyboardEvent, useState } from 'react';
import { api, errorMessage } from '../api';
import { acceleratorFrom } from './model';
import { Group, Row, TabHeader, type TabProps } from './parts';

const IN_APP: [string, string][] = [
  ['Search panel: restore the selected item', 'Enter'],
  ['Search panel: restore the whole group', 'CmdOrCtrl+Enter'],
  ['Search panel: restore into a new window', 'Alt+Enter'],
  ['Search panel: always keep the site', 'CmdOrCtrl+P'],
  ['Search panel: show details in the Library', 'CmdOrCtrl+I'],
  ['Close the panel or a drawer', 'Escape'],
  ['This list of shortcuts (Help menu, while a Decay window is in front)', 'CmdOrCtrl+/'],
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
      setHint('Use at least one of ⌘ / Ctrl / ⌥ / Alt with a key.');
    }
  };
  const use = () => {
    if (!candidate) return;
    api
      .setHotkey(candidate)
      .then(() => {
        notify('Search shortcut changed');
        setRecording(false);
        setCandidate(null);
      })
      .catch((e) => {
        // One message, next to the recorder (not the top banner too).
        setHint(`${errorMessage(e) || 'That shortcut is taken or not allowed.'} Try another.`);
      });
  };
  return (
    <>
      <TabHeader title="Shortcuts">Decay is keyboard-first.</TabHeader>
      <div className="grid gap-4">
        <Group title="Global shortcut">
          <Row label="Search the archive" description="Works from any app.">
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
                className="h-9 min-w-56 rounded-lg border border-line-strong bg-surface-2 px-3 text-left text-sm"
              >
                {recording ? (
                  candidate ? (
                    <Shortcut accelerator={candidate} />
                  ) : (
                    'Press the new shortcut…'
                  )
                ) : (
                  'Record a new shortcut'
                )}
              </button>
              {recording && candidate && (
                <Button size="sm" variant="primary" onClick={use}>
                  Use it
                </Button>
              )}
            </div>
            <p id="hotkey-hint" role={hint ? 'alert' : undefined} className="text-xs text-muted">
              {hint || (recording ? 'Esc cancels.' : 'Click, then press the keys together.')}
            </p>
          </div>
        </Group>
        <Group title="In the app">
          {IN_APP.map(([what, keys]) => (
            <Row key={what} label={what}>
              {keys === 'Enter' || keys === 'Escape' ? (
                <Kbd>{keys === 'Enter' ? '↵' : 'Esc'}</Kbd>
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
