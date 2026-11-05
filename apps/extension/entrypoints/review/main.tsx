import '@/assets/app.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ReviewPage } from './ReviewPage';

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <ReviewPage />
    </StrictMode>,
  );
}
