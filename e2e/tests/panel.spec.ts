import { expect, test } from '@playwright/test';

const email = process.env.SYNTHPANEL_E2E_EMAIL ?? 'admin@example.com';
const password = process.env.SYNTHPANEL_E2E_PASSWORD ?? 'hunter2password-77';

async function signIn(page: import('@playwright/test').Page) {
  await page.goto('/');
  await page.getByLabel('Email').fill(email);
  await page.getByLabel('Password').fill(password);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible();
}

test('login page is reachable and accessible', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Sign in to SynthPanel' })).toBeVisible();
  await expect(page.getByLabel('Email')).toBeVisible();
});

test('dashboard shows live metrics after login', async ({ page }) => {
  await signIn(page);
  await expect(page.getByText('Live server overview')).toBeVisible();
  await expect(page.getByText('CPU').first()).toBeVisible();
  await expect(page.getByRole('link', { name: 'Skip to content' })).toBeAttached();
});

test('site lifecycle through the UI', async ({ page }) => {
  await signIn(page);
  await page.getByRole('link', { name: 'Sites' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Sites' })).toBeVisible();

  const domain = `e2e-${Date.now()}.example.com`;
  await page.getByRole('button', { name: 'Add site' }).click();
  await page.getByLabel('Domain').fill(domain);
  await page.getByRole('button', { name: 'Create site' }).click();
  await expect(page.getByRole('button', { name: domain, exact: true })).toBeVisible();
});

test('backups page runs a backup', async ({ page }) => {
  await signIn(page);
  await page.getByRole('link', { name: 'Backups' }).click();
  await expect(page.getByRole('heading', { name: 'Backups' })).toBeVisible();
  await page.getByRole('button', { name: 'Run backup now' }).click();
  await expect(page.getByText(/Created synthpanel-/)).toBeVisible();
});

test('cron page creates a job', async ({ page }) => {
  await signIn(page);
  await page.getByRole('link', { name: 'Cron' }).click();
  await expect(page.getByRole('heading', { name: 'Cron' })).toBeVisible();
  await page.getByRole('button', { name: 'Add job' }).click();
  await page.getByLabel('Command').fill('echo e2e');
  await page.getByRole('button', { name: 'Create job' }).click();
  await expect(page.getByText('echo e2e')).toBeVisible();
});

test('alerts page creates a channel and a rule', async ({ page }) => {
  await signIn(page);
  await page.getByRole('link', { name: 'Alerts' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Alerts' })).toBeVisible();

  await page.getByRole('button', { name: 'Add channel' }).click();
  await page.getByLabel('Name').fill('e2e-webhook');
  await page.getByLabel('URL').fill('https://example.com/hook');
  await page.getByRole('button', { name: 'Create channel' }).click();
  await expect(page.getByText('The alert channel was created.')).toBeVisible();
  await expect(page.getByText('e2e-webhook', { exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Add rule' }).click();
  await page.getByLabel('Name').fill('e2e-rule');
  await page.getByRole('button', { name: 'Create rule' }).click();
  await expect(page.getByText('The alert rule was created.')).toBeVisible();
});

test('deployments page lists the deployment form', async ({ page }) => {
  await signIn(page);
  await page.getByRole('link', { name: 'Deployments' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Deployments' })).toBeVisible();
  await page.getByRole('button', { name: 'Add deployment' }).click();
  await expect(page.getByLabel('Repository')).toBeVisible();
});

test('php page lists the versions', async ({ page }) => {
  await signIn(page);
  await page.getByRole('link', { name: 'PHP' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'PHP' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'PHP 8.3' })).toBeVisible();
});

test('terminal page opens a shell', async ({ page }) => {
  await signIn(page);
  await page.getByRole('link', { name: 'Terminal' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Terminal' })).toBeVisible();
  await expect(page.getByLabel('Terminal', { exact: true })).toBeVisible();
  await expect(page.getByText(/Connected to/)).toBeVisible({ timeout: 20_000 });
});
