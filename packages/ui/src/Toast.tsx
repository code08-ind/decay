import { Leaf, X } from 'lucide-react';
import { type ReactNode, useEffect, useRef, useState } from 'react';
import { cx } from './cx';

/** Default lifetime (UI Design Guidelines §7: 30 s, paused on hover). */
export const TOAST_MS = 30_000;

/**
 * A live region that is always mounted (`aria-live="polite"`), so what appears inside it
 * is announced; screen readers often skip a region inserted together with its text.
 * Put toasts and status messages in it.
 */
export function LiveRegion(props: { children?: ReactNode; className?: string }) {
  return (
    <div role="status" aria-live="polite" className={props.className}>
      {props.children}
    </div>
  );
}

/**
 * A toast ("Archived 14 tabs · Undo"). Disappears after `durationMs`; the countdown
 * pauses while the pointer is over it or keyboard focus is inside it. Render it inside a
 * `LiveRegion`, and give each new toast its own `key` (a new toast restarts the timer).
 */
export function Toast(props: {
  children: ReactNode;
  action?: ReactNode;
  onDismiss: () => void;
  durationMs?: number;
  className?: string;
}) {
  const { children, action, onDismiss, durationMs = TOAST_MS, className } = props;
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const paused = hovered || focused;
  const left = useRef(durationMs);
  const dismiss = useRef(onDismiss);
  dismiss.current = onDismiss;
  useEffect(() => {
    if (paused) return;
    const started = Date.now();
    const timer = setTimeout(() => dismiss.current(), left.current);
    return () => {
      clearTimeout(timer);
      left.current = Math.max(0, left.current - (Date.now() - started));
    };
  }, [paused]);
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: hover/focus only pause the timer
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={(e) => {
        // Moving between the toast's own buttons keeps it paused.
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(false);
      }}
      className={cx(
        'flex w-[360px] max-w-full items-center gap-3 rounded-2xl border border-line bg-surface p-3.5 shadow-popover',
        className,
      )}
    >
      <span
        aria-hidden
        className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-feature text-lime"
      >
        <Leaf size={18} />
      </span>
      <div className="min-w-0 flex-1 text-xs leading-snug text-ink">{children}</div>
      {action}
      <button
        type="button"
        aria-label="Dismiss"
        onClick={onDismiss}
        className="flex size-6 shrink-0 items-center justify-center rounded-full text-muted hover:bg-surface-2 hover:text-ink"
      >
        <X size={14} aria-hidden />
      </button>
    </div>
  );
}
