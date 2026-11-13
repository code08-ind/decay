import { defineConfig } from 'vitest/config';
import { WxtVitest } from 'wxt/testing/vitest-plugin';

export default defineConfig({
  plugins: [WxtVitest()],
  test: {
    environment: 'node',
    setupFiles: ['fake-indexeddb/auto'],
    // `pnpm coverage`: CI fails below these (target 80–85 % for the main features).
    coverage: {
      provider: 'v8',
      // Feature code. Entry points (background wiring, page roots) are covered by the
      // end-to-end QA in a real browser.
      include: ['lib/**', 'components/**'],
      exclude: ['**/*.test.*', 'lib/**/test-utils.ts'],
      thresholds: { lines: 80, statements: 80, functions: 80, branches: 75 },
    },
  },
});
