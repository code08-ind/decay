import './app.css';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { followThemeSetting } from './theme';
import { Library } from './views/Library';
import { Panel } from './views/Panel';
import { Settings } from './views/Settings';

// One bundle for every window; the window's label picks the view.
const label = getCurrentWindow().label;
const View = label === 'panel' ? Panel : label === 'settings' ? Settings : Library;
if (label === 'panel') document.documentElement.classList.add('panel-window');
followThemeSetting();

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <View />
    </StrictMode>,
  );
}
