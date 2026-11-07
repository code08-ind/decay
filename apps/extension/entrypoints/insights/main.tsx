import '@/assets/app.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { InsightsPage } from './InsightsPage';

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <InsightsPage />
    </StrictMode>,
  );
}
