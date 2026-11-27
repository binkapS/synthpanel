import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { request } from '@playwright/test';

/**
 * Log in once for the whole suite and hand the cookies to every test through
 * storageState. The UI has no login form in the DOM and the core rate-limits
 * /auth/login to 10 attempts per five minutes, so logging in per test is not
 * viable. Tests that need an anonymous session override storageState locally.
 */
const here = path.dirname(fileURLToPath(import.meta.url));
export const statePath =
  process.env.SYNTHPANEL_E2E_STATE_FILE ?? path.join(here, '.auth', 'state.json');

export default async function globalSetup(): Promise<void> {
  const baseURL = process.env.SYNTHPANEL_E2E_BASE_URL ?? 'https://127.0.0.1:8443';
  const email = process.env.SYNTHPANEL_E2E_EMAIL ?? 'admin@example.com';
  const password = process.env.SYNTHPANEL_E2E_PASSWORD ?? 'hunter2password-77';

  await mkdir(path.dirname(statePath), { recursive: true });
  const context = await request.newContext({ baseURL, ignoreHTTPSErrors: true });
  try {
    const response = await context.post('/api/v1/auth/login', { data: { email, password } });
    if (!response.ok()) {
      throw new Error(`e2e setup login failed (${response.status()}): ${await response.text()}`);
    }
    await context.storageState({ path: statePath });
  } finally {
    await context.dispose();
  }
}
