import './app.css';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { api } from './api';
import { followThemeSetting } from './theme';
import { Library } from './views/Library';
import { Onboarding } from './views/Onboarding';
import { Panel } from './views/Panel';
import { Settings } from './views/Settings';

// One bundle for every window; the window's label picks the view.
const label = getCurrentWindow().label;
const View =
  label === 'panel'
    ? Panel
    : label === 'settings'
      ? Settings
      : label === 'onboarding'
        ? Onboarding
        : Library;
if (label === 'panel') document.documentElement.classList.add('panel-window');
followThemeSetting();

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

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <View />
    </StrictMode>,
  );
}
