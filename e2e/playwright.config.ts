import { defineConfig, devices } from '@playwright/test';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const baseURL = process.env.SYNTHPANEL_E2E_BASE_URL ?? 'https://127.0.0.1:8443';
// run.sh points this at its throwaway work dir; bare `npx playwright test`
// falls back to a local path under e2e/.auth.
const storageState =
  process.env.SYNTHPANEL_E2E_STATE_FILE ??
  path.join(path.dirname(fileURLToPath(import.meta.url)), '.auth', 'state.json');

export default defineConfig({
  testDir: './tests',
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['html', { open: 'never' }]] : [['list']],
  globalSetup: './global-setup.ts',
  use: {
    baseURL,
    storageState,
    ignoreHTTPSErrors: true,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
