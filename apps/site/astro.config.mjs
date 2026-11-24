// Decay's website (P4.4): https://decay.aryangarg.com, a static Astro site deployed by
// Vercel from this folder (Docs/Release.md "Website"). The app's Help menu opens /docs.
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'astro/config';

export default defineConfig({
  site: 'https://decay.aryangarg.com',
  output: 'static',
  trailingSlash: 'never',
  build: { format: 'file' },
  vite: { plugins: [tailwindcss()] },
});
