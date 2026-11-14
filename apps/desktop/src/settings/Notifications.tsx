import { Switch } from '@decay/ui';
import { useEffect, useState } from 'react';
import { Group, Row, TabHeader, type TabProps } from './parts';

type Switches = {
  summary: boolean;
  sweep: boolean;
  sweepBigOnly: boolean;
  browser: boolean;
  permission: boolean;
};

/** The switches from the stored `notifications` (an older `health` covers both alerts). */
export function switchesOf(value: unknown): Switches {
  const n = (value && typeof value === 'object' ? value : {}) as Record<string, unknown>;
  const flag = (key: string, fallback: boolean) =>
    typeof n[key] === 'boolean' ? (n[key] as boolean) : fallback;
  const health = flag('health', true);
  return {
    summary: flag('summary', true),
    sweep: flag('sweep', false),
    sweepBigOnly: flag('sweepBigOnly', false),
    browser: flag('browser', health),
    permission: flag('permission', health),
  };
}

// Notifications (Designs/09f, F10 / F25): the few moments Decay may speak.
export function Notifications(props: TabProps) {
  const { settings, save } = props;
  const stored = switchesOf(settings?.notifications);
  // Shown at once and sent whole: two quick toggles can't undo each other (the second
  // save starts from the first, not from the settings broadcast still on its way).
  const [n, setN] = useState(stored);
  const key = JSON.stringify(stored);
  // biome-ignore lint/correctness/useExhaustiveDependencies: compare by value
  useEffect(() => setN(stored), [key]);
  const time =
    typeof settings?.daily_summary_time === 'string' ? settings.daily_summary_time : '09:00';
  const set = (patch: Partial<Switches>, done: string) => {
    const next = { ...n, ...patch };
    setN(next);
    save({ notifications: next }, done);
  };
  return (
    <>
      <TabHeader title="Notifications">
        Decay stays quiet. Choose the few moments it's allowed to speak.
      </TabHeader>
      <div className="grid gap-4">
        <Group title="Daily summary">
          <Switch
            label="Morning summary"
            description="What was archived yesterday, and anything worth a look. Only when something happened."
            checked={n.summary}
            onChange={(on) =>
              set({ summary: on }, on ? 'Morning summary on' : 'Morning summary off')
            }
          />
          <Row label="Time" description="Local time; it waits until a call or meeting ends.">
            <input
              type="time"
              aria-label="Morning summary time"
              value={time}
              disabled={!n.summary}
              onChange={(e) => {
                if (/^\d{2}:\d{2}$/.test(e.target.value)) {
                  save({ daily_summary_time: e.target.value }, `Summary at ${e.target.value}`);
                }
              }}
              className="h-9 rounded-lg border border-line bg-surface-2 px-3 text-sm disabled:opacity-50"
            />
          </Row>
        </Group>
        <Group title="Sweeps">
          <Switch
            label="Notify after each sweep"
            description="A short note when a scheduled sweep archives tabs or hides windows."
            checked={n.sweep}
            onChange={(on) => set({ sweep: on }, on ? 'Sweep notices on' : 'Sweep notices off')}
          />
          <Switch
            label="Only when more than 10 items are tidied"
            description="Skip the note for small cleanups."
            checked={n.sweepBigOnly}
            disabled={!n.sweep}
            onChange={(on) => set({ sweepBigOnly: on }, 'Saved')}
          />
        </Group>
        <Group title="Health">
          <Switch
            label="Tell me if a browser disconnects"
            description="After a minute without the Decay extension (its tabs aren't tidied meanwhile)."
            checked={n.browser}
            onChange={(on) => set({ browser: on }, 'Saved')}
          />
          <Switch
            label="Tell me if a permission is turned off"
            description="If Accessibility, needed to hide windows, is revoked."
            checked={n.permission}
            onChange={(on) => set({ permission: on }, 'Saved')}
          />
        </Group>
        <p className="text-xs text-muted">
          Nothing pops up during calls and meetings, and macOS Focus / Do Not Disturb apply as for
          any app.
        </p>
      </div>
    </>
  );
}
