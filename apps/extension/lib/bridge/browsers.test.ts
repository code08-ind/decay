import { describe, expect, it } from 'vitest';
import config, { FIREFOX_EXTENSION_ID } from '../../wxt.config';
import { browserKind } from './chrome';

// F14: one extension source, a Chromium build and a Firefox build.
describe('browsers', () => {
  it('names the browser for the app', () => {
    expect(browserKind('com.google.Chrome')).toBe('chrome');
    expect(browserKind('com.microsoft.Edge')).toBe('edge');
    expect(browserKind('com.brave.Browser')).toBe('brave');
    expect(browserKind('org.mozilla.firefox')).toBe('firefox');
  });

  it('builds a Firefox manifest without Chromium-only permissions, with its fixed id', () => {
    const manifest = config.manifest as (env: { browser: string }) => Record<string, unknown>;
    const firefox = manifest({ browser: 'firefox' });
    const chrome = manifest({ browser: 'chrome' });
    expect(firefox.permissions).not.toContain('favicon');
    expect(firefox.permissions).not.toContain('tabGroups');
    expect(chrome.permissions).toContain('favicon');
    expect(chrome.permissions).toContain('tabGroups');
    expect(firefox.browser_specific_settings).toEqual({
      gecko: {
        id: FIREFOX_EXTENSION_ID,
        strict_min_version: '128.0',
        data_collection_permissions: { required: ['none'] },
      },
    });
    expect(firefox.minimum_chrome_version).toBeUndefined();
    expect(chrome.minimum_chrome_version).toBe('120');
    // The desktop app's native host accepts exactly this id.
    expect(FIREFOX_EXTENSION_ID).toBe('decay@decay-app');
  });
});
