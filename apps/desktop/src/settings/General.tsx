import {
  isThemePreference,
  SegmentedControl,
  Shortcut,
  Switch,
  type ThemePreference,
} from '@decay/ui';
import { api, type Mode } from '../api';
import { LANGUAGES, t } from '../strings';
import { useStatus } from '../useStatus';
import { Group, Row, TabHeader, type TabProps } from './parts';

/** A language's name in that language ("English", "Français"); pseudo-locales by tag. */
function languageName(tag: string): string {
  if (tag === 'en-XA' || tag === 'ar-XB') return tag; // i18n-ignore: development only
  try {
    return new Intl.DisplayNames([tag], { type: 'language' }).of(tag) ?? tag;
  } catch {
    return tag;
  }
}

// General (Designs/09a): launch at login, theme, mode, the search shortcut.
export function General(props: TabProps & { go: (tab: string) => void }) {
  const { settings, save, notify, fail } = props;
  const status = useStatus();
  const theme: ThemePreference = isThemePreference(settings?.theme) ? settings.theme : 'system';
  const hotkey = typeof settings?.hotkey === 'string' ? settings.hotkey : '';
  const locale = typeof settings?.locale === 'string' ? settings.locale : 'system';
  // A stored language without an option (from an import, or a pseudo-locale in a release
  // build) is listed too, so the menu says what's set.
  const offered = [...LANGUAGES, ...(import.meta.env.DEV ? ['en-XA', 'ar-XB'] : [])];
  const languages =
    locale === 'system' || offered.includes(locale) ? offered : [...offered, locale];
  const mode: Mode = status?.mode ?? (settings?.mode as Mode) ?? 'shadow';
  const setMode = (m: 'shadow' | 'automatic') => {
    api
      .setMode(m)
      .then(() => notify(m === 'automatic' ? t('general.automaticOn') : t('general.backToShadow')))
      .catch(fail);
  };
  return (
    <>
      <TabHeader title={t('settings.tab.general')}>{t('general.subtitle')}</TabHeader>
      <div className="grid gap-4">
        <Group title={t('general.startup')}>
          <Switch
            label={t('general.launch')}
            description={t('general.launch.text')}
            checked={settings?.launch_at_login !== false}
            onChange={(on) =>
              api
                .setLaunchAtLogin(on)
                .then(() => notify(on ? t('general.launchOn') : t('general.launchOff')))
                .catch(fail)
            }
          />
        </Group>
        <Group title={t('general.appearance')}>
          <Row label={t('general.theme')} description={t('general.theme.text')}>
            <SegmentedControl<ThemePreference>
              label={t('general.theme')}
              value={theme}
              onChange={(value) => save({ theme: value })}
              options={[
                { value: 'light', label: t('general.theme.light') },
                { value: 'dark', label: t('general.theme.dark') },
                { value: 'system', label: t('general.theme.system') },
              ]}
            />
          </Row>
          <Row label={t('general.language')} description={t('general.language.text')}>
            <select
              aria-label={t('general.language')}
              value={locale}
              onChange={(e) => save({ locale: e.target.value }, t('common.saved'))}
              className="h-8 rounded-lg border border-line bg-surface-2 px-2 text-[13px]"
            >
              <option value="system">{t('general.language.system')}</option>
              {languages.map((tag) => (
                <option key={tag} value={tag}>
                  {languageName(tag)}
                </option>
              ))}
            </select>
          </Row>
        </Group>
        <Group title={t('general.mode')}>
          <Row
            label={t('general.mode')}
            description={mode === 'paused' ? t('general.mode.paused') : t('general.mode.text')}
          >
            {mode === 'paused' ? (
              <button
                type="button"
                className="h-8 rounded-full border border-line-strong bg-cta px-4 text-[13px] text-cta-ink"
                onClick={() =>
                  api
                    .resume()
                    .then(() => notify(t('general.resumed')))
                    .catch(fail)
                }
              >
                {t('general.resume')}
              </button>
            ) : (
              <SegmentedControl<'shadow' | 'automatic'>
                label={t('general.mode')}
                value={mode}
                onChange={setMode}
                options={[
                  { value: 'shadow', label: t('mode.shadow') },
                  { value: 'automatic', label: t('mode.automatic') },
                ]}
              />
            )}
          </Row>
        </Group>
        <Group title={t('general.shortcut')}>
          <Row label={t('general.search')} description={t('general.search.text')}>
            <span className="flex items-center gap-3">
              {hotkey && <Shortcut accelerator={hotkey} />}
              <button
                type="button"
                className="text-[13px] text-accent-text underline underline-offset-2"
                onClick={() => props.go('shortcuts')}
              >
                {t('general.change')}
              </button>
            </span>
          </Row>
        </Group>
      </div>
    </>
  );
}
