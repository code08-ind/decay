import { cx } from './cx';
import { t } from './strings';

export type KeyPlatform = 'mac' | 'windows';

/** The platform the UI runs on (from the user agent; Windows otherwise). */
export function detectPlatform(ua: string = globalThis.navigator?.userAgent ?? ''): KeyPlatform {
  return /Mac|iPhone|iPad/.test(ua) ? 'mac' : 'windows';
}

type KeyName = Parameters<typeof t>[0];

// Each key: its cap, and its spoken name as a catalog key (read in the locale's words).
const MAC: Record<string, [glyph: string, name: KeyName]> = {
  super: ['⌘', 'key.command'],
  cmd: ['⌘', 'key.command'],
  command: ['⌘', 'key.command'],
  cmdorctrl: ['⌘', 'key.command'],
  commandorcontrol: ['⌘', 'key.command'],
  alt: ['⌥', 'key.option'],
  option: ['⌥', 'key.option'],
  shift: ['⇧', 'key.shift'],
  ctrl: ['⌃', 'key.control'],
  control: ['⌃', 'key.control'],
  enter: ['↵', 'key.return'],
  return: ['↵', 'key.return'],
  backspace: ['⌫', 'key.delete'],
  escape: ['Esc', 'key.escape'],
  esc: ['Esc', 'key.escape'],
};

const WIN: Record<string, [label: string, name: KeyName]> = {
  super: ['Win', 'key.windows'],
  cmd: ['Win', 'key.windows'],
  command: ['Win', 'key.windows'],
  cmdorctrl: ['Ctrl', 'key.control'],
  commandorcontrol: ['Ctrl', 'key.control'],
  alt: ['Alt', 'key.alt'],
  option: ['Alt', 'key.alt'],
  shift: ['Shift', 'key.shift'],
  ctrl: ['Ctrl', 'key.control'],
  control: ['Ctrl', 'key.control'],
  enter: ['Enter', 'key.enter'],
  return: ['Enter', 'key.enter'],
  backspace: ['Backspace', 'key.backspace'],
  escape: ['Esc', 'key.escape'],
  esc: ['Esc', 'key.escape'],
};

const ORDER_MAC = ['⌃', '⌥', '⇧', '⌘'];

/**
 * An accelerator ("Alt+Super+D", "CmdOrCtrl+Enter") as keycaps for `platform`, plus a
 * spoken label. macOS lists modifiers in its own order (⌃⌥⇧⌘).
 */
export function formatShortcut(
  accelerator: string,
  platform: KeyPlatform = detectPlatform(),
): { keys: string[]; label: string } {
  const table = platform === 'mac' ? MAC : WIN;
  const parts = accelerator
    .split('+')
    .map((p) => p.trim())
    .filter(Boolean)
    .map((p): [cap: string, spoken: string] => {
      const known = table[p.toLowerCase()];
      if (known) return [known[0], t(known[1])];
      const key = p.length === 1 ? p.toUpperCase() : p; // other keys are read as they are
      return [key, key];
    });
  if (platform === 'mac') {
    const rank = (k: string) => {
      const i = ORDER_MAC.indexOf(k);
      return i === -1 ? ORDER_MAC.length : i;
    };
    parts.sort((a, b) => rank(a[0]) - rank(b[0]));
  }
  return { keys: parts.map((p) => p[0]), label: parts.map((p) => p[1]).join(' ') };
}

/** One keycap. */
export function Kbd({ children, className }: { children: string; className?: string }) {
  return (
    <kbd
      className={cx(
        // The system font: DM Mono has no ⌘ ⌥ ⇧ ⌃ glyphs (the fallback renders them tiny).
        'inline-flex h-6 min-w-[26px] items-center justify-center rounded-lg border border-line-strong bg-surface-2 px-1.5 font-[system-ui] text-xs text-ink shadow-[0_1px_0_rgba(0,0,0,0.08)]',
        className,
      )}
    >
      {children}
    </kbd>
  );
}

/** A shortcut as keycaps, read out as words ("Option Command D"). */
export function Shortcut(props: {
  accelerator: string;
  platform?: KeyPlatform;
  className?: string;
}) {
  const { keys, label } = formatShortcut(props.accelerator, props.platform);
  return (
    <span
      className={cx('inline-flex items-center gap-1', props.className)}
      role="img"
      aria-label={label}
    >
      {keys.map((k, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: keys may repeat; order is fixed
        <Kbd key={i}>{k}</Kbd>
      ))}
    </span>
  );
}
