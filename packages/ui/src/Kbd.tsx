import { cx } from './cx';

export type KeyPlatform = 'mac' | 'windows';

/** The platform the UI runs on (from the user agent; Windows otherwise). */
export function detectPlatform(ua: string = globalThis.navigator?.userAgent ?? ''): KeyPlatform {
  return /Mac|iPhone|iPad/.test(ua) ? 'mac' : 'windows';
}

const MAC: Record<string, [glyph: string, name: string]> = {
  super: ['⌘', 'Command'],
  cmd: ['⌘', 'Command'],
  command: ['⌘', 'Command'],
  cmdorctrl: ['⌘', 'Command'],
  commandorcontrol: ['⌘', 'Command'],
  alt: ['⌥', 'Option'],
  option: ['⌥', 'Option'],
  shift: ['⇧', 'Shift'],
  ctrl: ['⌃', 'Control'],
  control: ['⌃', 'Control'],
  enter: ['↵', 'Return'],
  return: ['↵', 'Return'],
  backspace: ['⌫', 'Delete'],
  escape: ['Esc', 'Escape'],
  esc: ['Esc', 'Escape'],
};

const WIN: Record<string, [label: string, name: string]> = {
  super: ['Win', 'Windows'],
  cmd: ['Win', 'Windows'],
  command: ['Win', 'Windows'],
  cmdorctrl: ['Ctrl', 'Control'],
  commandorcontrol: ['Ctrl', 'Control'],
  alt: ['Alt', 'Alt'],
  option: ['Alt', 'Alt'],
  shift: ['Shift', 'Shift'],
  ctrl: ['Ctrl', 'Control'],
  control: ['Ctrl', 'Control'],
  enter: ['Enter', 'Enter'],
  return: ['Enter', 'Enter'],
  backspace: ['Backspace', 'Backspace'],
  escape: ['Esc', 'Escape'],
  esc: ['Esc', 'Escape'],
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
    .map((p) => {
      const known = table[p.toLowerCase()];
      if (known) return known;
      const key = p.length === 1 ? p.toUpperCase() : p;
      return [key, key] as [string, string];
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
