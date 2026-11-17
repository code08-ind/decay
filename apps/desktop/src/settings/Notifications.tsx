import { Switch } from '@decay/ui';
import { useEffect, useState } from 'react';
import { t } from '../strings';
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
      <TabHeader title={t('settings.tab.notifications')}>{t('notifications.subtitle')}</TabHeader>
      <div className="grid gap-4">
        <Group title={t('notifications.daily')}>
          <Switch
            label={t('notifications.morning')}
            description={t('notifications.morning.text')}
            checked={n.summary}
            onChange={(on) =>
              set(
                { summary: on },
                on ? t('notifications.morningOn') : t('notifications.morningOff'),
              )
            }
          />
          <Row label={t('notifications.time')} description={t('notifications.time.text')}>
            <input
              type="time"
              aria-label={t('notifications.time.label')}
              value={time}
              disabled={!n.summary}
              onChange={(e) => {
                if (/^\d{2}:\d{2}$/.test(e.target.value)) {
                  save(
                    { daily_summary_time: e.target.value },
                    t('notifications.time.saved', { time: e.target.value }),
                  );
                }
              }}
              className="h-9 rounded-lg border border-line bg-surface-2 px-3 text-sm disabled:opacity-50"
            />
          </Row>
        </Group>
        <Group title={t('notifications.sweeps')}>
          <Switch
            label={t('notifications.sweep')}
            description={t('notifications.sweep.text')}
            checked={n.sweep}
            onChange={(on) =>
              set({ sweep: on }, on ? t('notifications.sweepOn') : t('notifications.sweepOff'))
            }
          />
          <Switch
            label={t('notifications.big')}
            description={t('notifications.big.text')}
            checked={n.sweepBigOnly}
            disabled={!n.sweep}
            onChange={(on) => set({ sweepBigOnly: on }, t('common.saved'))}
          />
        </Group>
        <Group title={t('notifications.health')}>
          <Switch
            label={t('notifications.browser')}
            description={t('notifications.browser.text')}
            checked={n.browser}
            onChange={(on) => set({ browser: on }, t('common.saved'))}
          />
          <Switch
            label={t('notifications.permission')}
            description={t('notifications.permission.text')}
            checked={n.permission}
            onChange={(on) => set({ permission: on }, t('common.saved'))}
          />
        </Group>
        <p className="text-xs text-muted">{t('notifications.quiet')}</p>
      </div>
    </>
  );
}
