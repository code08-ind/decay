import { cx } from '@decay/ui';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Copy, Minus, Square, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { DecayLeaf } from './DecayLeaf';
import { t } from './strings';

/** What the window's page was told about its chrome (Rust `shell::page_query`). */
export interface ChromeFlags {
  /** Windows: no system frame, so the page draws the title bar. */
  custom: boolean;
  /** A system backdrop (Mica, Acrylic) shows through the page's background. */
  backdrop: 'mica' | 'acrylic' | null;
}

export function chromeFlags(search: string): ChromeFlags {
  const q = new URLSearchParams(search);
  const b = q.get('backdrop');
  return {
    custom: q.get('chrome') === 'custom',
    backdrop: b === 'mica' || b === 'acrylic' ? b : null,
  };
}

/** Marks the page for the CSS (`html[data-chrome]`, `html[data-backdrop]`). */
export function applyChrome(flags: ChromeFlags, root: HTMLElement = document.documentElement) {
  if (flags.custom) root.dataset.chrome = 'custom';
  if (flags.backdrop) root.dataset.backdrop = flags.backdrop;
}

const CAPTION =
  'grid h-8 w-[46px] place-items-center text-ink transition-colors hover:bg-ink/10 active:bg-ink/15';

/**
 * The Windows title bar (UI Design Guidelines §8): the leaf and the window's title (a drag
 * region: drag to move, double-click to maximise), then the caption buttons, 46×32, at the
 * top-right. The buttons stay out of the Tab order, like the system's (Alt+Space and
 * Alt+F4 work as ever).
 */
export function TitleBar(props: { resizable?: boolean }) {
  const win = getCurrentWindow();
  const [maximized, setMaximized] = useState(false);
  const [title, setTitle] = useState('');
  useEffect(() => {
    const w = getCurrentWindow();
    w.title()
      .then(setTitle)
      .catch(() => {});
    const sync = () => {
      w.isMaximized()
        .then(setMaximized)
        .catch(() => {});
    };
    sync();
    const off = w.onResized(sync);
    return () => {
      off.then((f) => f()).catch(() => {});
    };
  }, []);
  const resizable = props.resizable ?? true;
  return (
    <div
      data-tauri-drag-region
      className="flex h-8 shrink-0 select-none items-center bg-transparent ps-3"
    >
      <DecayLeaf data-tauri-drag-region size={16} className="text-accent" aria-hidden />
      <span data-tauri-drag-region className="ms-2 min-w-0 flex-1 truncate font-serif text-sm">
        {title}
      </span>
      {resizable && (
        <button
          type="button"
          tabIndex={-1}
          aria-label={t('titlebar.minimize')}
          className={CAPTION}
          onClick={() => win.minimize()}
        >
          <Minus size={16} strokeWidth={1.25} aria-hidden />
        </button>
      )}
      {resizable && (
        <button
          type="button"
          tabIndex={-1}
          aria-label={maximized ? t('titlebar.restore') : t('titlebar.maximize')}
          className={CAPTION}
          onClick={() => win.toggleMaximize()}
        >
          {maximized ? (
            <Copy size={13} strokeWidth={1.25} className="-scale-x-100" aria-hidden />
          ) : (
            <Square size={13} strokeWidth={1.25} aria-hidden />
          )}
        </button>
      )}
      <button
        type="button"
        tabIndex={-1}
        aria-label={t('common.close')}
        className={cx(CAPTION, 'hover:bg-[#c42b1c] hover:text-white active:bg-[#c42b1c]/90')}
        onClick={() => win.close()}
      >
        <X size={16} strokeWidth={1.25} aria-hidden />
      </button>
    </div>
  );
}
