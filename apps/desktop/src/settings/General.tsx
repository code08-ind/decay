import {
  isThemePreference,
  SegmentedControl,
  Shortcut,
  Switch,
  type ThemePreference,
} from '@decay/ui';
import { api, type Mode } from '../api';
import { useStatus } from '../useStatus';
import { Group, Row, TabHeader, type TabProps } from './parts';

// General (Designs/09a): launch at login, theme, mode, the search shortcut.
export function General(props: TabProps & { go: (tab: string) => void }) {
  const { settings, save, notify, fail } = props;
  const status = useStatus();
  const theme: ThemePreference = isThemePreference(settings?.theme) ? settings.theme : 'system';
  const hotkey = typeof settings?.hotkey === 'string' ? settings.hotkey : '';
  const mode: Mode = status?.mode ?? (settings?.mode as Mode) ?? 'shadow';
  const setMode = (m: 'shadow' | 'automatic') => {
    api
      .setMode(m)
      .then(() => notify(m === 'automatic' ? 'Automatic mode is on' : 'Back in shadow mode'))
      .catch(fail);
  };
  return (
    <>
      <TabHeader title="General">How Decay starts, looks and behaves.</TabHeader>
      <div className="grid gap-4">
        <Group title="Startup">
          <Switch
            label="Launch Decay at login"
            description="Start quietly in the background when you log in."
            checked={settings?.launch_at_login !== false}
            onChange={(on) =>
              api
                .setLaunchAtLogin(on)
                .then(() => notify(on ? 'Decay starts at login' : "Decay doesn't start at login"))
                .catch(fail)
            }
          />
        </Group>
        <Group title="Appearance">
          <Row label="Theme" description="Application colour scheme.">
            <SegmentedControl<ThemePreference>
              label="Theme"
              value={theme}
              onChange={(t) => save({ theme: t })}
              options={[
                { value: 'light', label: 'Light' },
                { value: 'dark', label: 'Dark' },
                { value: 'system', label: 'Match system' },
              ]}
            />
          </Row>
        </Group>
        <Group title="Mode">
          <Row
            label="Mode"
            description={
              mode === 'paused'
                ? 'Paused from the menu bar: nothing ages or is archived until you resume.'
                : 'Automatic mode archives on schedule. Shadow mode only shows what it would do.'
            }
          >
            {mode === 'paused' ? (
              <button
                type="button"
                className="h-8 rounded-full border border-line-strong bg-cta px-4 text-[13px] text-cta-ink"
                onClick={() =>
                  api
                    .resume()
                    .then(() => notify('Resumed'))
                    .catch(fail)
                }
              >
                Resume
              </button>
            ) : (
              <SegmentedControl<'shadow' | 'automatic'>
                label="Mode"
                value={mode}
                onChange={setMode}
                options={[
                  { value: 'shadow', label: 'Shadow mode' },
                  { value: 'automatic', label: 'Automatic' },
                ]}
              />
            )}
          </Row>
        </Group>
        <Group title="Global shortcut">
          <Row label="Search the archive" description="Opens the search panel from anywhere.">
            <span className="flex items-center gap-3">
              {hotkey && <Shortcut accelerator={hotkey} />}
              <button
                type="button"
                className="text-[13px] text-accent-text underline underline-offset-2"
                onClick={() => props.go('shortcuts')}
              >
                Change
              </button>
            </span>
          </Row>
        </Group>
      </div>
    </>
  );
}
