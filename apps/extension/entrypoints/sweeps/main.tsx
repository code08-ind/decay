import '@/assets/app.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { SweepsPage } from './SweepsPage';

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <SweepsPage />
    </StrictMode>,
  );
}
