import { describe, expect, it } from 'vitest';
import { applyTheme, cx, detectPlatform, effectiveTheme, formatShortcut } from './index';

describe('formatShortcut', () => {
  it('uses macOS glyphs in macOS order', () => {
    expect(formatShortcut('Super+Alt+D', 'mac')).toEqual({
      keys: ['⌥', '⌘', 'D'],
      label: 'Option Command D',
    });
    expect(formatShortcut('CmdOrCtrl+Shift+enter', 'mac').keys).toEqual(['⇧', '⌘', '↵']);
    expect(formatShortcut('Ctrl+Alt+Shift+Super+k', 'mac').keys).toEqual(['⌃', '⌥', '⇧', '⌘', 'K']);
  });

  it('uses words on Windows, in the given order', () => {
    expect(formatShortcut('Ctrl+Alt+D', 'windows')).toEqual({
      keys: ['Ctrl', 'Alt', 'D'],
      label: 'Control Alt D',
    });
    expect(formatShortcut('CmdOrCtrl+Backspace', 'windows').keys).toEqual(['Ctrl', 'Backspace']);
    expect(formatShortcut(' Super + F1 ', 'windows').keys).toEqual(['Win', 'F1']);
  });

  it('detects the platform', () => {
    expect(detectPlatform('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)')).toBe('mac');
    expect(detectPlatform('Mozilla/5.0 (Windows NT 10.0; Win64; x64)')).toBe('windows');
  });
});

describe('theme', () => {
  it('sets or clears data-theme', () => {
    const root = document.createElement('html');
    expect(applyTheme('dark', root)).toBe('dark');
    expect(root.dataset.theme).toBe('dark');
    expect(applyTheme('system', root)).toBe('system');
    expect(root.dataset.theme).toBeUndefined();
    expect(applyTheme('neon', root)).toBe('system');
  });

  it('resolves system through the OS', () => {
    expect(effectiveTheme('system', true)).toBe('dark');
    expect(effectiveTheme('system', false)).toBe('light');
    expect(effectiveTheme('light', true)).toBe('light');
  });
});

it('cx joins truthy classes', () => {
  expect(cx('a', false, null, undefined, 'b')).toBe('a b');
});
