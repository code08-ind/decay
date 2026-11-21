import './app.css';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { api } from './api';
import { followLocaleSetting, Localized } from './locale';
import { applyChrome, chromeFlags, TitleBar } from './TitleBar';
import { followThemeSetting } from './theme';
import { Library } from './views/Library';
import { Onboarding } from './views/Onboarding';
import { Panel } from './views/Panel';
import { Popover } from './views/Popover';
import { Settings } from './views/Settings';

// One bundle for every window; the window's label picks the view.
const label = getCurrentWindow().label;
const View =
  label === 'panel'
    ? Panel
    : label === 'popover'
      ? Popover
      : label === 'settings'
        ? Settings
        : label === 'onboarding'
          ? Onboarding
          : Library;
if (label === 'panel') document.documentElement.classList.add('panel-window');
// Windows: the frame and backdrop Rust gave this window (`shell::page_query`).
const chrome = chromeFlags(window.location.search);
applyChrome(chrome);
followThemeSetting();
const languageKnown = followLocaleSetting();

// Errors in this window go to Decay's log (scrubbed there; at most 100 a run).
const report = (message: string, stack?: string) => {
  api.logUiError(message, stack).catch(() => {});
};
window.addEventListener('error', (e) => report(String(e.message), e.error?.stack));
window.addEventListener('unhandledrejection', (e) => {
  const r = e.reason;
  if (r instanceof Error) report(r.message, r.stack);
  // A rejected command: `{ code, message }`.
  else if (r && typeof r === 'object' && 'message' in r)
    report(`${'code' in r ? `${String(r.code)}: ` : ''}${String(r.message)}`);
  else report(String(r));
});

// Drawn once the language is known, so the window doesn't appear in one and redraw in another.
const root = document.getElementById('root');
if (root) {
  void languageKnown.then(() =>
    createRoot(root).render(
      <StrictMode>
        <Localized
          render={() =>
            chrome.custom ? (
              <div className="flex h-screen flex-col">
                <TitleBar resizable={label !== 'onboarding'} />
                <div className="min-h-0 flex-1 overflow-auto">
                  <View />
                </div>
              </div>
            ) : (
              <View />
            )
          }
        />
      </StrictMode>,
    ),
  );
}
