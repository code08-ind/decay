import { Button } from '@decay/ui';
import {
  AppWindow,
  ArrowRight,
  ArrowRightLeft,
  BarChart3,
  CalendarDays,
  Leaf,
  type LucideIcon,
  Search,
  Star,
  Video,
  X,
} from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { errorMessage } from '../api';
import { t } from '../strings';
import { formatDate, type NoteIcon, type ReleaseNotes } from './releases';

const ICONS: Record<NoteIcon, LucideIcon> = {
  windows: AppWindow,
  summary: CalendarDays,
  transfer: ArrowRightLeft,
  insights: BarChart3,
  meeting: Video,
  search: Search,
};

// What's New (Designs/16): once after an update, or from Help → What's New. "Got it"
// marks the version seen; "Later" (Escape, ×) shows it again next time Decay starts.
// `latest`: the running version has no notes; these are the newest there are.
export function WhatsNew(props: {
  notes: ReleaseNotes;
  latest?: boolean;
  onGotIt: () => void;
  onLater: () => void;
  onChangelog: () => Promise<unknown>;
}) {
  const { notes } = props;
  const [error, setError] = useState('');
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const gotItRef = useRef<HTMLButtonElement>(null);
  const onLater = useRef(props.onLater);
  onLater.current = props.onLater;
  // On open: focus "Got it", keep Tab inside; on close, focus goes back.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    gotItRef.current?.focus();
    // Captured first and kept from the page behind (a drawer closes on Escape too).
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopImmediatePropagation();
        onLater.current();
        return;
      }
      if (e.key !== 'Tab' || !dialogRef.current) return;
      e.stopImmediatePropagation();
      const focusable = dialogRef.current.querySelectorAll<HTMLElement>('button');
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !dialogRef.current.contains(active))) {
        e.preventDefault();
        last?.focus();
      } else if (!e.shiftKey && (active === last || !dialogRef.current.contains(active))) {
        e.preventDefault();
        first?.focus();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      opener?.focus?.();
    };
  }, []);
  return (
    <div className="fixed inset-0 z-30 grid place-items-center bg-black/40 p-4">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex max-h-[90vh] w-full max-w-2xl flex-col rounded-card bg-canvas p-6 shadow-xl"
      >
        <div className="flex items-start gap-4">
          <span
            aria-hidden
            className="grid size-12 shrink-0 place-items-center rounded-full border border-line bg-surface text-accent"
          >
            <Leaf size={22} />
          </span>
          <div className="min-w-0 flex-1">
            <p className="flex flex-wrap items-center gap-3 text-sm text-muted">
              <span className="rounded-full bg-chip px-2.5 py-0.5 text-chip-ink">
                {props.latest
                  ? t('whatsNew.latest', { version: notes.version })
                  : t('whatsNew.updatedTo', { version: notes.version })}
              </span>
              {formatDate(notes.date)}
            </p>
            <h2 id={titleId} className="mt-1 font-serif text-3xl">
              {t('whatsNew.title')}
            </h2>
          </div>
          <button
            type="button"
            onClick={props.onLater}
            className="grid size-9 shrink-0 place-items-center rounded-full bg-surface-2 text-muted hover:text-ink"
          >
            <X size={16} aria-hidden />
            <span className="sr-only">{t('common.close')}</span>
          </button>
        </div>
        <div className="mt-5 grid min-h-0 gap-3 overflow-auto">
          {notes.major && (
            <section className="rounded-card bg-feature p-5 text-feature-ink">
              <p className="flex items-center gap-2 font-mono text-xs tracking-widest text-feature-sub uppercase">
                <Star size={14} aria-hidden /> {t('whatsNew.major')}
              </p>
              <h3 className="mt-2 font-serif text-2xl">{notes.major.title}</h3>
              <p className="mt-1 text-feature-sub">{notes.major.text}</p>
            </section>
          )}
          <ul className="grid gap-3">
            {notes.items.map((item) => {
              const Icon = ICONS[item.icon];
              return (
                <li
                  key={item.title}
                  className="flex items-start gap-4 rounded-card border border-line bg-surface p-4"
                >
                  <span
                    aria-hidden
                    className="grid size-10 shrink-0 place-items-center rounded-full bg-surface-2 text-accent"
                  >
                    <Icon size={18} />
                  </span>
                  <div>
                    <h3 className="font-medium">{item.title}</h3>
                    <p className="text-sm text-muted">{item.text}</p>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
        <div className="mt-5 flex flex-wrap items-center justify-between gap-3 border-t border-line pt-4">
          <div>
            <button
              type="button"
              onClick={() => {
                setError('');
                props.onChangelog().catch((e) => setError(errorMessage(e)));
              }}
              className="inline-flex items-center gap-1.5 text-sm text-muted hover:text-ink"
            >
              {t('whatsNew.changelog')}{' '}
              <ArrowRight className="rtl:-scale-x-100" size={14} aria-hidden />
            </button>
            {error && (
              <p role="alert" className="text-sm text-danger">
                {error}
              </p>
            )}
          </div>
          <div className="flex gap-2">
            <Button onClick={props.onLater}>{t('whatsNew.later')}</Button>
            <Button ref={gotItRef} variant="primary" onClick={props.onGotIt}>
              {t('whatsNew.gotIt')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
