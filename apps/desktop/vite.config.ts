import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

// Tauri loads http://localhost:1420 in development (tauri.conf.json devUrl) and dist/ in
// release builds.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: 'safari16', outDir: 'dist', emptyOutDir: true },
  test: { environment: 'node' },
});
