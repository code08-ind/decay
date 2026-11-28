import {
  Banner,
  Button,
  Chip,
  cx,
  detectPlatform,
  formatList,
  LiveRegion,
  logicalArrow,
  richText,
  Shortcut,
  StatusDot,
  Switch,
} from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  ArrowRight,
  CalendarDays,
  Check,
  Clock,
  ExternalLink,
  Globe,
  Info,
  Lock,
  Play,
  Search,
  Shield,
  ShieldCheck,
} from 'lucide-react';
import {
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from 'react';
import {
  api,
  type DetectedBrowser,
  errorMessage,
  type Preset,
  type WindowManagement,
} from '../api';
import { DecayLeaf } from '../DecayLeaf';
import {
  back,
  hostProblem,
  next,
  PRESETS,
  rowDetail,
  rowState,
  STEPS,
  sortRows,
  stepKicker,
  stepTitle,
} from '../onboarding/model';
import { t } from '../strings';
import { useStatus } from '../useStatus';

const platform = detectPlatform();
const DEFAULT_HOTKEY = platform === 'mac' ? 'Alt+Super+D' : 'Ctrl+Alt+D';
/** How often the Connect step re-checks the browsers (installing an extension takes a while). */
const DETECT_MS = 2000;

// Onboarding (F21, Designs/03a–e): six steps beside a step list. Nothing is stored until
// "Start shadow mode" (complete_onboarding); closing the window early leaves Decay in
// shadow mode, and the next launch shows onboarding again.
export function Onboarding() {
  const [step, setStep] = useState(0);
  const [preset, setPreset] = useState<Preset>('balanced');
  const [windows, setWindows] = useState<WindowManagement>('hide');
  const [completed, setCompleted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const heading = useRef<HTMLHeadingElement>(null);

  // Screen readers and keyboard users start each step at its heading.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs on step change
  useEffect(() => {
    heading.current?.focus();
  }, [step]);

  const go = (to: number) => {
    setError('');
    setStep(to);
  };
  const finish = async () => {
    setBusy(true);
    setError('');
    try {
      await api.completeOnboarding({ preset, windowManagement: windows, launchAtLogin: true });
      setCompleted(true);
      setStep(next(step));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const id = STEPS[step]?.id ?? 'welcome';
  const primary: { label: string; onClick: () => void } =
    id === 'welcome'
      ? { label: t('onboarding.getStarted'), onClick: () => go(next(step)) }
      : id === 'shadow'
        ? { label: t('onboarding.startShadow'), onClick: finish }
        : { label: t('onboarding.continue'), onClick: () => go(next(step)) };

  return (
    <main className="flex h-(--view-h) gap-4 bg-canvas p-4 text-ink">
      <Sidebar step={step} />
      <section
        aria-labelledby="step-title"
        className="flex min-w-0 flex-1 flex-col rounded-card border border-line bg-surface px-8 pt-7 pb-5"
      >
        <div className="min-h-0 flex-1 overflow-y-auto">
          {id === 'welcome' && <Welcome heading={heading} />}
          {id === 'pace' && <Pace heading={heading} value={preset} onChange={setPreset} />}
          {id === 'connect' && <Connect heading={heading} />}
          {id === 'windows' && (
            <WindowAccess heading={heading} value={windows} onChange={setWindows} />
          )}
          {id === 'shadow' && <Shadow heading={heading} />}
          {id === 'done' && <Done heading={heading} />}
          {error && (
            <Banner tone="danger" className="mt-4">
              {error}
            </Banner>
          )}
        </div>
        {id !== 'done' && (
          <footer className="mt-4 flex items-center gap-3 border-t border-line pt-4">
            {step > 0 && !completed ? (
              <Button variant="secondary" size="lg" onClick={() => go(back(step, completed))}>
                {t('onboarding.back')}
              </Button>
            ) : (
              <span />
            )}
            <Dots step={step} />
            <Button
              variant="primary"
              size="lg"
              className="ms-auto"
              loading={busy}
              onClick={primary.onClick}
            >
              {primary.label}
              <ArrowRight className="rtl:-scale-x-100" size={16} aria-hidden />
            </Button>
          </footer>
        )}
      </section>
    </main>
  );
}

function Sidebar({ step }: { step: number }) {
  return (
    <nav
      aria-label={t('onboarding.steps')}
      className="flex w-60 shrink-0 flex-col rounded-card bg-feature p-5 text-feature-ink [--focus:var(--feature-text)]"
    >
      <div className="mb-6 flex items-center gap-2">
        <span className="grid size-8 place-items-center rounded-lg bg-feature-tile">
          <DecayLeaf size={16} aria-hidden />
        </span>
        <span className="font-serif text-xl">Decay</span>
      </div>
      <ol className="grid gap-3 text-sm">
        {STEPS.map((s, i) => {
          const done = i < step;
          const current = i === step;
          return (
            <li
              key={s.id}
              aria-current={current ? 'step' : undefined}
              className={cx(
                'flex items-center gap-3',
                current ? 'font-semibold' : done ? 'text-feature-ink' : 'text-feature-sub',
              )}
            >
              <span
                aria-hidden
                className={cx(
                  'grid size-6 place-items-center rounded-full border text-[11px]',
                  current
                    ? 'border-feature-ink bg-feature-ink text-feature'
                    : 'border-feature-tile-line',
                  done && 'bg-feature-tile',
                )}
              >
                {done ? <Check size={12} /> : i + 1}
              </span>
              <span>
                {stepTitle(s.id)}
                {done && <span className="sr-only"> {t('onboarding.stepDone')}</span>}
              </span>
            </li>
          );
        })}
      </ol>
      <div className="mt-auto rounded-xl border border-feature-tile-line bg-feature-tile p-3 text-xs leading-relaxed text-feature-sub">
        <p className="mb-1 flex items-center gap-1.5 font-semibold tracking-wide text-feature-ink uppercase">
          <ShieldCheck size={14} aria-hidden /> {t('onboarding.guarantee')}
        </p>
        {t('onboarding.guarantee.text')}
      </div>
    </nav>
  );
}

function Dots({ step }: { step: number }) {
  return (
    <div aria-hidden className="mx-auto flex items-center gap-1.5">
      {STEPS.map((s, i) => (
        <span
          key={s.id}
          className={cx(
            'h-1.5 rounded-full',
            i === step ? 'w-6 bg-accent-dot' : 'w-1.5 bg-line-strong',
          )}
        />
      ))}
    </div>
  );
}

function Header(props: {
  step: number;
  title: string;
  heading: RefObject<HTMLHeadingElement | null>;
  children?: ReactNode;
}) {
  const stepId = STEPS[props.step]?.id;
  const kicker = stepId ? stepKicker(stepId) : undefined;
  return (
    <header className="mb-5">
      <p className="mb-2 text-[11px] font-semibold tracking-widest text-muted uppercase">
        {t('onboarding.stepOf', { n: props.step + 1, total: STEPS.length })}
        {kicker && (
          <span className="font-normal tracking-normal normal-case">
            {t('onboarding.kickerSep', { kicker })}
          </span>
        )}
      </p>
      <h1
        id="step-title"
        ref={props.heading}
        tabIndex={-1}
        className="font-serif text-[28px] leading-tight font-medium outline-none"
      >
        {props.title}
      </h1>
      {props.children && (
        <p className="mt-3 text-sm leading-relaxed text-muted">{props.children}</p>
      )}
    </header>
  );
}

type HeadingRef = { heading: RefObject<HTMLHeadingElement | null> };

function Point(props: { icon: ReactNode; title: string; children: ReactNode }) {
  return (
    <li className="flex items-center gap-4 rounded-xl border border-line bg-surface px-4 py-3">
      <span
        aria-hidden
        className="grid size-9 shrink-0 place-items-center rounded-full bg-chip text-accent-text"
      >
        {props.icon}
      </span>
      <span>
        <span className="block text-sm font-semibold">{props.title}</span>
        <span className="block text-[13px] text-muted">{props.children}</span>
      </span>
    </li>
  );
}

function useHotkey(): string {
  const [hotkey, setHotkey] = useState(DEFAULT_HOTKEY);
  useEffect(() => {
    api
      .getSettings()
      .then((s) => typeof s.hotkey === 'string' && setHotkey(s.hotkey))
      .catch(() => {});
  }, []);
  return hotkey;
}

function Welcome({ heading }: HeadingRef) {
  const hotkey = useHotkey();
  return (
    <>
      <Header step={0} heading={heading} title={t('welcome.title')}>
        {t('welcome.text')}
      </Header>
      <ul className="grid gap-2.5">
        <Point icon={<Clock size={16} />} title={t('welcome.active')}>
          {t('welcome.active.text')}
        </Point>
        <Point icon={<Shield size={16} />} title={t('welcome.safe')}>
          {t('welcome.safe.text')}
        </Point>
        <Point icon={<Search size={16} />} title={t('welcome.back')}>
          {richText((p) => t('welcome.back.text', p), {
            keys: <Shortcut accelerator={hotkey} />,
          })}
        </Point>
      </ul>
      <p className="mt-5 flex items-center gap-2 text-[13px] text-muted">
        <Lock size={14} aria-hidden /> {t('welcome.local')}
      </p>
    </>
  );
}

/** Cards as a radio group: arrow keys move and select (like SegmentedControl). */
function useRadioKeys<T>(values: T[], value: T, onChange: (v: T) => void) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKeyDown = (e: KeyboardEvent) => {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[logicalArrow(e.key)];
    if (step === undefined) return;
    e.preventDefault();
    const i = (values.indexOf(value) + step + values.length) % values.length;
    const v = values[i];
    if (v === undefined) return;
    onChange(v);
    refs.current[i]?.focus();
  };
  return { refs, onKeyDown };
}

function Pace(props: HeadingRef & { value: Preset; onChange: (p: Preset) => void }) {
  const { refs, onKeyDown } = useRadioKeys(
    PRESETS.map((p) => p.id),
    props.value,
    props.onChange,
  );
  return (
    <>
      <Header step={1} heading={props.heading} title={t('pace.title')}>
        {t('pace.text')}
      </Header>
      <div
        role="radiogroup"
        aria-label={t('pace.label')}
        className="grid grid-cols-3 gap-3"
        onKeyDown={onKeyDown}
      >
        {PRESETS.map((p, i) => {
          const on = p.id === props.value;
          return (
            // biome-ignore lint/a11y/useSemanticElements: cards with the radio pattern (roving tabindex, arrow keys)
            <button
              key={p.id}
              ref={(el) => {
                refs.current[i] = el;
              }}
              type="button"
              role="radio"
              aria-checked={on}
              tabIndex={on ? 0 : -1}
              onClick={() => props.onChange(p.id)}
              className={cx(
                'relative flex flex-col rounded-xl border p-4 text-start',
                on
                  ? 'border-accent bg-accent-soft ring-1 ring-accent'
                  : 'border-line bg-surface hover:bg-surface-2',
              )}
            >
              {on && (
                <span
                  aria-hidden
                  className="absolute top-2.5 end-2.5 grid size-5 place-items-center rounded-full bg-accent-dot text-primary-ink"
                >
                  <Check size={12} />
                </span>
              )}
              <span className="font-serif text-lg font-medium">{t(p.name)}</span>
              {p.id === 'balanced' && (
                <Chip tone="accent" className="mt-1 self-start">
                  {t('pace.recommended')}
                </Chip>
              )}
              <span className="mt-2 mb-4 text-[13px] text-muted">{t(p.blurb)}</span>
              {/* Spans: a radio button may only hold phrasing content. */}
              <span className="mt-auto grid gap-1 rounded-lg border border-line bg-surface p-2.5 text-xs">
                {(
                  [
                    [t('pace.stale'), p.hours.stale],
                    [t('pace.hide'), p.hours.hidden],
                    [t('pace.archive'), p.hours.archive],
                  ] as const
                ).map(([k, v]) => (
                  <span key={k} className="flex justify-between">
                    <span className="text-muted">{k}</span>
                    <span className="font-medium">{t('pace.hours', { n: v })}</span>
                  </span>
                ))}
              </span>
            </button>
          );
        })}
      </div>
      <p className="mt-4 text-[13px] text-muted">{t('pace.note')}</p>
    </>
  );
}

function Connect({ heading }: HeadingRef) {
  const [rows, setRows] = useState<DetectedBrowser[] | null>(null);
  const [hostError, setHostError] = useState('');
  const [opened, setOpened] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [detectError, setDetectError] = useState('');
  useEffect(() => {
    let live = true;
    // Polls can overlap: only the newest answer counts.
    let asked = 0;
    let shown = 0;
    const refresh = () => {
      const n = ++asked;
      api
        .detectBrowsers()
        .then((r) => {
          if (!live || n < shown) return;
          shown = n;
          setRows(r);
          setDetectError('');
        })
        .catch((e) => {
          if (!live || n < shown) return;
          shown = n;
          setDetectError(errorMessage(e));
        });
    };
    // Install the host first (a browser installed since start-up), then look.
    api
      .installNativeHosts()
      .catch((e) => live && setHostError(hostProblem(e)))
      .finally(refresh);
    const timer = setInterval(refresh, DETECT_MS);
    const off = listen('adapter:health', refresh);
    return () => {
      live = false;
      clearInterval(timer);
      off.then((f) => f()).catch(() => {});
    };
  }, []);
  const open = (b: DetectedBrowser) => {
    setError('');
    api
      .openExtensionStore(b.id)
      .then(() => setOpened(b.id))
      .catch((e) => setError(errorMessage(e)));
  };
  const sorted = rows ? sortRows(rows) : [];
  const connected = sorted.filter((b) => b.extensionConnected);
  return (
    <>
      <Header step={2} heading={heading} title={t('connect.title')}>
        {t('connect.text')}
      </Header>
      {hostError && (
        <Banner tone="warning" className="mb-3">
          {hostError}
        </Banner>
      )}
      {detectError && (
        <Banner tone="danger" className="mb-3">
          {t('connect.cantLook', { error: detectError })}
        </Banner>
      )}
      {error && (
        <Banner tone="danger" className="mb-3">
          {error}
        </Banner>
      )}
      <LiveRegion className="sr-only">
        {connected.length > 0
          ? t('connect.connected', { names: formatList(connected.map((b) => b.name)) })
          : ''}
      </LiveRegion>
      {rows === null ? (
        !detectError && <p className="text-sm text-muted">{t('connect.looking')}</p>
      ) : (
        <ul className="grid gap-2.5">
          {sorted.map((b) => {
            const state = rowState(b);
            return (
              <li
                key={b.id}
                className={cx(
                  'flex items-center gap-4 rounded-xl border border-line p-3.5',
                  state === 'missing' ? 'bg-canvas text-muted' : 'bg-surface',
                )}
              >
                <span
                  aria-hidden
                  className="grid size-9 shrink-0 place-items-center rounded-lg border border-line bg-surface-2"
                >
                  <Globe size={16} />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-semibold">{b.name}</span>
                  <span className="block text-xs text-muted">
                    {opened === b.id && state === 'install'
                      ? t('connect.addIn', { name: b.name })
                      : rowDetail(b)}
                  </span>
                </span>
                {state === 'connected' && <StatusDot status="connected" />}
                {state === 'install' && (
                  <Button
                    size="sm"
                    onClick={() => open(b)}
                    icon={<ExternalLink className="rtl:-scale-x-100" size={12} aria-hidden />}
                    aria-label={t('connect.installIn', { name: b.name })}
                  >
                    {t('connect.install')}
                  </Button>
                )}
                {state === 'missing' && <span className="text-xs">{t('connect.notFound')}</span>}
              </li>
            );
          })}
        </ul>
      )}
      {rows !== null && connected.length === 0 && (
        <p className="mt-4 text-[13px] text-muted">{t('connect.none')}</p>
      )}
    </>
  );
}

function WindowAccess(
  props: HeadingRef & { value: WindowManagement; onChange: (v: WindowManagement) => void },
) {
  const status = useStatus();
  const [note, setNote] = useState('');
  const mac = platform === 'mac';
  const allowed = status?.accessibility === true;
  const { refs, onKeyDown } = useRadioKeys<WindowManagement>(
    ['hide', 'off'],
    props.value,
    props.onChange,
  );
  const ask = () =>
    api
      .requestAccessibility()
      .then((r) => setNote(r.opened === 'prompt' ? t('windows.prompt') : t('windows.settings')))
      .catch((e) => setNote(errorMessage(e)));
  const option = (
    value: WindowManagement,
    i: number,
    title: string,
    blurb: string,
    extra?: ReactNode,
  ) => {
    const on = props.value === value;
    return (
      <div
        className={cx(
          'relative rounded-xl border p-4',
          on ? 'border-accent bg-accent-soft ring-1 ring-accent' : 'border-line bg-surface',
        )}
      >
        {/* biome-ignore lint/a11y/useSemanticElements: cards with the radio pattern (roving tabindex, arrow keys) */}
        <button
          ref={(el) => {
            refs.current[i] = el;
          }}
          type="button"
          role="radio"
          aria-checked={on}
          tabIndex={on ? 0 : -1}
          onClick={() => props.onChange(value)}
          className="block w-full text-start after:absolute after:inset-0 after:content-['']"
        >
          <span className="flex items-center gap-2 font-serif text-lg font-medium">
            {title}
            {value === 'hide' && (
              <Chip tone="accent" className="font-sans">
                {t('windows.recommended')}
              </Chip>
            )}
          </span>
          <span className="mt-1 block text-[13px] text-muted">{blurb}</span>
        </button>
        {on && (
          <span
            aria-hidden
            className="absolute top-2.5 end-2.5 grid size-5 place-items-center rounded-full bg-accent-dot text-primary-ink"
          >
            <Check size={12} />
          </span>
        )}
        {extra}
      </div>
    );
  };
  return (
    <>
      <Header step={3} heading={props.heading} title={t('windows.title')}>
        {mac ? t('windows.text.mac') : t('windows.text')}
      </Header>
      <div
        role="radiogroup"
        aria-label={t('windows.label')}
        className="grid gap-3"
        onKeyDown={onKeyDown}
      >
        {option(
          'hide',
          0,
          t('windows.both'),
          mac ? t('windows.both.mac') : t('windows.both.text'),
          mac && props.value === 'hide' ? (
            // Above the radio's click area, so its buttons work on their own.
            // biome-ignore lint/a11y/noStaticElementInteractions: only stops arrow keys from reaching the radio group
            <div
              className="relative z-10 mt-3 flex items-center gap-3 rounded-lg border border-line bg-surface p-3"
              onKeyDown={(e) => e.stopPropagation()}
            >
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-semibold">{t('windows.ax')}</span>
                <span className="block text-xs text-muted">{t('windows.ax.where')}</span>
              </span>
              {allowed ? (
                <StatusDot status="connected" label={t('windows.ax.allowed')} />
              ) : (
                <Button
                  size="sm"
                  onClick={ask}
                  icon={<ExternalLink className="rtl:-scale-x-100" size={12} aria-hidden />}
                >
                  {t('windows.ax.open')}
                </Button>
              )}
            </div>
          ) : undefined,
        )}
        {option('off', 1, t('windows.tabsOnly'), t('windows.tabsOnly.text'))}
      </div>
      <LiveRegion className="sr-only">{allowed ? t('windows.ax.live') : note}</LiveRegion>
      {mac && (
        <p className="mt-3 flex items-center gap-2 text-[13px] text-muted">
          <Info size={14} aria-hidden className="shrink-0" />
          {props.value === 'hide' && !allowed
            ? note || t('windows.continueNow')
            : t('windows.revoke')}
        </p>
      )}
    </>
  );
}

function Shadow({ heading }: HeadingRef) {
  return (
    <>
      <Header step={4} heading={heading} title={t('shadowStep.title')}>
        {t('shadowStep.text')}
      </Header>
      <div className="rounded-xl border border-line bg-surface p-4">
        <div aria-hidden className="flex gap-1.5">
          {[0, 1, 2, 3, 4, 5, 6].map((d) => (
            <span
              key={d}
              className={cx(
                'h-1.5 flex-1 rounded-full',
                d === 0 ? 'bg-accent-dot' : 'bg-surface-3',
              )}
            />
          ))}
        </div>
        <p className="mt-2 flex justify-between text-xs text-muted">
          <span>{t('shadowStep.days')}</span>
          <span className="font-semibold text-accent-text">{t('shadowStep.decide')}</span>
        </p>
        <ul className="mt-3 grid gap-3 border-t border-line pt-3 text-sm">
          <li className="flex items-center gap-3">
            <CalendarDays size={16} aria-hidden className="text-accent-text" />{' '}
            {t('shadowStep.list')}
          </li>
          <li className="flex items-center gap-3">
            <ShieldCheck size={16} aria-hidden className="text-accent-text" />{' '}
            {t('shadowStep.protect')}
          </li>
          <li className="flex items-center gap-3">
            <Play size={16} aria-hidden className="text-accent-text" /> {t('shadowStep.auto')}
          </li>
        </ul>
      </div>
    </>
  );
}

function Done({ heading }: HeadingRef) {
  const hotkey = useHotkey();
  const [login, setLogin] = useState(true);
  const [error, setError] = useState('');
  const toggle = (on: boolean) => {
    setLogin(on);
    setError('');
    api.setLaunchAtLogin(on).catch((e) => {
      setLogin(!on);
      setError(errorMessage(e));
    });
  };
  const open = async () => {
    try {
      await api.showWindow('library');
      await getCurrentWindow().close();
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return (
    <div className="mx-auto flex max-w-sm flex-col items-center pt-4 text-center">
      <span
        aria-hidden
        className="mb-4 grid size-16 place-items-center rounded-full bg-chip text-accent-text"
      >
        <DecayLeaf size={28} />
      </span>
      <h1
        id="step-title"
        ref={heading}
        tabIndex={-1}
        className="font-serif text-[28px] leading-tight font-medium outline-none"
      >
        {t('done.title')}
      </h1>
      <p className="mt-2 text-sm text-muted">{t('done.text', { place: platform })}</p>
      <div className="mt-5 w-full rounded-xl border border-line bg-surface p-4">
        <Shortcut accelerator={hotkey} className="text-base" />
        <p className="mt-2 text-xs text-muted">{t('done.search')}</p>
      </div>
      <div className="mt-3 w-full rounded-xl border border-line bg-surface p-4 text-start">
        <Switch checked={login} onChange={toggle} label={t('done.login')} />
      </div>
      {error && (
        <Banner tone="danger" className="mt-3 w-full text-start">
          {error}
        </Banner>
      )}
      <Button variant="primary" size="lg" className="mt-6 w-full" onClick={open}>
        {t('done.open')}
      </Button>
    </div>
  );
}
