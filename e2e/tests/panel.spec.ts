import { expect, test, type Page } from '@playwright/test';

// Slint renders the whole panel into a single <canvas>: there is no semantic
// DOM for getByLabel/getByRole. These tests therefore drive the app through
// `window.synthpanel_e2e` (installed by slint-loader.js only for `?e2e=1`).
//
// What is verified today, without touching Rust:
//   - the WASM app boots, installs the JS bridge and issues requests itself;
//   - the HTTP API journeys the Slint callbacks use (login, sites, cron,
//     backups, alerts, deployments, PHP, logs, terminal + its WebSocket).
//
// What cannot be verified without Rust support is kept as a test that skips
// itself until the hook exists (see skipWithoutHooks below). Needed hooks:
//   window.synthpanel_e2e_login({email, password})      -> void
//   window.synthpanel_e2e_navigate(pageName: string)    -> void
//   window.synthpanel_e2e_snapshot()                    -> JSON string/object
//   window.synthpanel_e2e_ready()                       -> void (optional)
// See the report in the migration PR for the exact Rust signatures.

const email = process.env.SYNTHPANEL_E2E_EMAIL ?? 'admin@example.com';
const password = process.env.SYNTHPANEL_E2E_PASSWORD ?? 'hunter2password-77';

type HookName = 'login' | 'navigate' | 'snapshot';
type Envelope = { status: number; body: any };
type RequestEntry = { method: string; path: string; status: number; at: number; durationMs: number };
type Snapshot = {
  ready?: boolean;
  authenticated: boolean;
  loginVisible?: boolean;
  loading?: boolean;
  currentPage: string;
  sessionEmail?: string;
  dashboardStatus?: string;
  sites?: string[];
  sitesStatus?: string;
  cronCommands?: string[];
  cronStatus?: string;
  backups?: string[];
  backupsStatus?: string;
  alertChannels?: string[];
  alertRules?: string[];
  alertsStatus?: string;
  deployments?: string[];
  phpVersions?: string[];
  logLines?: string[];
  logsStatus?: string;
  terminalId?: string;
  terminalShell?: string;
  terminalStatus?: string;
};

type E2eShim = {
  enabled: boolean;
  bootState(): string;
  hasHook(name: HookName): boolean;
  requestLog(): RequestEntry[];
  clearRequestLog(): void;
  apiRequest(method: string, path: string, body?: unknown): Promise<Envelope>;
  apiOk(method: string, path: string, body?: unknown): Promise<any>;
  waitForReady(timeoutMs?: number): Promise<void>;
  waitForRequest(method: string, path: string, timeoutMs?: number): Promise<RequestEntry>;
  login(email: string, password: string): void;
  navigate(page: string): void;
  snapshot(): Snapshot;
  openSocket(id: number, path: string): void;
  socketState(id: number): number | null;
  waitForSocket(id: number, readyState?: number, timeoutMs?: number): Promise<void>;
  closeSocket(id: number): void;
};

declare global {
  interface Window {
    synthpanel_e2e?: E2eShim;
  }
}

async function openApp(page: Page): Promise<void> {
  await page.goto('/?e2e=1');
  await expect(page.locator('#canvas')).toBeVisible();
  await expect(page.locator('#boot-error')).toBeHidden();
  await page.evaluate(async () => {
    const e2e = window.synthpanel_e2e;
    if (!e2e?.enabled) throw new Error('e2e shim is missing; was the UI built with the ?e2e=1 loader?');
    await e2e.waitForReady(20_000);
  });
}

async function api(page: Page, method: string, path: string, body?: unknown): Promise<Envelope> {
  return page.evaluate(
    async ({ method: m, path: p, body: b }) => {
      const e2e = window.synthpanel_e2e;
      if (!e2e?.enabled) throw new Error('e2e shim is missing');
      return e2e.apiRequest(m, p, b);
    },
    { method, path, body },
  );
}

/** Assert that a request came from the app's bridge, not from the test itself. */
async function appRequest(page: Page, method: string, path: string): Promise<RequestEntry> {
  return page.evaluate(
    ({ method: m, path: p }) => window.synthpanel_e2e!.waitForRequest(m, p, 15_000),
    { method, path },
  );
}

async function expectSession(page: Page): Promise<void> {
  const me = await api(page, 'GET', '/api/v1/auth/me');
  expect(me.status).toBe(200);
  expect(me.body).toBeTruthy();
}

async function hasHook(page: Page, name: HookName): Promise<boolean> {
  return page.evaluate((n) => window.synthpanel_e2e!.hasHook(n), name);
}

/**
 * Keep UI-state journeys honest: without a Rust hook the canvas cannot tell us
 * which page is shown or what its models contain, so mark the test skipped
 * until the parent adds the hook. It will start running automatically.
 */
async function skipWithoutHooks(page: Page, hooks: HookName[], reason: string): Promise<void> {
  const present = await page.evaluate(
    (names) => names.map((name) => window.synthpanel_e2e!.hasHook(name)),
    hooks,
  );
  const missing = hooks.filter((_, i) => !present[i]);
  test.skip(
    missing.length > 0,
    `${reason} Missing Rust hooks: ${missing.map((name) => `window.synthpanel_e2e_${name}()`).join(', ')}.`,
  );
}

async function navigate(page: Page, name: string): Promise<void> {
  await page.evaluate((p) => window.synthpanel_e2e!.navigate(p), name);
  await expectSession(page);
}

async function snapshot(page: Page): Promise<Snapshot> {
  return page.evaluate(() => window.synthpanel_e2e!.snapshot());
}

test('slint canvas boots and the Rust bridge drives requests', async ({ page }) => {
  await openApp(page);
  expect(await page.evaluate(() => window.synthpanel_e2e!.bootState())).toBe('running');
  // The app calls /auth/me from load_session(); the test never calls it here.
  const request = await appRequest(page, 'GET', '/api/v1/auth/me');
  expect(request.status).toBe(200); // global setup supplies the session cookie
});

test.describe('unauthenticated', () => {
  // Override the suite-wide session cookie from global setup.
  test.use({ storageState: { cookies: [], origins: [] } });

  test('login screen is reachable and the session is rejected', async ({ page }) => {
    await openApp(page);
    await appRequest(page, 'GET', '/api/v1/auth/me');
    const me = await api(page, 'GET', '/api/v1/auth/me');
    expect(me.status).toBe(401);
  });

  test('login form is rendered on the canvas', async ({ page }) => {
    await openApp(page);
    await skipWithoutHooks(
      page,
      ['snapshot'],
      'Slint has no semantic DOM: the login form can only be asserted through app state.',
    );
    const state = await snapshot(page);
    expect(state.authenticated).toBe(false);
    expect(state.loginVisible).toBe(true);
  });

  test('login establishes the session used by the UI', async ({ page }) => {
    await openApp(page);
    const result = await api(page, 'POST', '/api/v1/auth/login', { email, password });
    expect(result.status).toBe(200);
    expect(result.body.requires_2fa).toBe(false);
    await expectSession(page);
  });

  test('login callback updates Slint app state', async ({ page }) => {
    await openApp(page);
    await skipWithoutHooks(
      page,
      ['login', 'snapshot'],
      'Driving the Slint login callback and reading authenticated state needs Rust hooks.',
    );
    await page.evaluate(
      (credentials) => window.synthpanel_e2e!.login(credentials.email, credentials.password),
      { email, password },
    );
    await appRequest(page, 'POST', '/api/v1/auth/login');
    await expect
      .poll(async () => (await snapshot(page)).authenticated, { timeout: 15_000 })
      .toBe(true);
  });
});

test('dashboard shows live metrics after login', async ({ page }) => {
  await openApp(page);
  await expectSession(page);
  const metrics = await api(page, 'GET', '/api/v1/metrics/snapshot');
  expect(metrics.status).toBe(200);
  expect(metrics.body).toBeTruthy();
  const range = await api(page, 'GET', '/api/v1/metrics/range?range=24h');
  expect(range.status).toBe(200);
  expect(Array.isArray(range.body.points)).toBe(true);
  const services = await api(page, 'GET', '/api/v1/services');
  expect(services.status).toBe(200);
});

test('dashboard page loads metrics into the canvas', async ({ page }) => {
  await openApp(page);
  await skipWithoutHooks(
    page,
    ['navigate', 'snapshot'],
    'The dashboard page and its models are only observable through app state.',
  );
  await navigate(page, 'dashboard');
  await appRequest(page, 'GET', '/api/v1/metrics/snapshot');
  const state = await snapshot(page);
  expect(state.currentPage).toBe('dashboard');
  expect(state.dashboardStatus).toBe('');
});

test('site lifecycle through the API the Slint UI uses', async ({ page }) => {
  await openApp(page);
  await expectSession(page);

  const domain = `e2e-${Date.now()}.example.com`;
  const created = await api(page, 'POST', '/api/v1/sites', {
    domain,
    aliases: [],
    template: 'static',
    php_version: null,
    ssl_mode: 'none',
    document_root: '',
  });
  expect(created.status).toBe(200);
  const siteId = created.body.site?.id;
  expect(siteId).toBeTruthy();
  expect(created.body.site.domain).toBe(domain);

  const listed = await api(page, 'GET', '/api/v1/sites');
  expect(listed.status).toBe(200);
  expect(listed.body.sites.map((site: any) => site.domain)).toContain(domain);

  const detail = await api(page, 'GET', `/api/v1/sites/${siteId}`);
  expect(detail.status).toBe(200);
  // The detail endpoint returns SiteExtras { site, nginx, php }.
  expect(detail.body.site.domain).toBe(domain);

  expect((await api(page, 'DELETE', `/api/v1/sites/${siteId}`)).status).toBe(200);
  const after = await api(page, 'GET', '/api/v1/sites');
  expect(after.body.sites.map((site: any) => site.domain)).not.toContain(domain);
});

test('site lifecycle through the UI', async ({ page }) => {
  await openApp(page);
  await skipWithoutHooks(
    page,
    ['navigate', 'snapshot'],
    'Site creation is a Slint callback; its form and list cannot be reached without a navigation/state hook.',
  );
  const domain = `e2e-ui-${Date.now()}.example.com`;
  await navigate(page, 'sites');
  await appRequest(page, 'GET', '/api/v1/sites');
  const before = await snapshot(page);
  expect(before.currentPage).toBe('sites');
  // The shim cannot fill canvas inputs; creation stays an API step. The hook
  // must refresh models when the data changes for this assertion to work.
  const created = await api(page, 'POST', '/api/v1/sites', {
    domain,
    aliases: [],
    template: 'static',
    php_version: null,
    ssl_mode: 'none',
    document_root: '',
  });
  expect(created.status).toBe(200);
  // Re-navigate so the Rust page loader refreshes the Slint model.
  await navigate(page, 'sites');
  await expect.poll(async () => (await snapshot(page)).sites, { timeout: 15_000 }).toContain(domain);
});

test('cron page creates a job through the API the UI uses', async ({ page }) => {
  await openApp(page);
  await expectSession(page);

  const command = `echo e2e-${Date.now()}`;
  const created = await api(page, 'POST', '/api/v1/cron', {
    expression: '*/5 * * * *',
    command,
    user: null,
    site_id: null,
  });
  expect(created.status).toBe(200);
  const jobId = created.body.id;
  expect(jobId).toBeTruthy();

  const listed = await api(page, 'GET', '/api/v1/cron');
  expect(listed.status).toBe(200);
  expect(listed.body.jobs.map((job: any) => job.command)).toContain(command);
  const runs = await api(page, 'GET', `/api/v1/cron/${jobId}/runs?limit=10`);
  expect(runs.status).toBe(200);

  expect((await api(page, 'DELETE', `/api/v1/cron/${jobId}`)).status).toBe(200);
  const after = await api(page, 'GET', '/api/v1/cron');
  expect(after.body.jobs.map((job: any) => job.command)).not.toContain(command);
});

test('cron page creates a job through the UI', async ({ page }) => {
  await openApp(page);
  await skipWithoutHooks(
    page,
    ['navigate', 'snapshot'],
    'The cron form and job list live in Slint models, not in the DOM.',
  );
  await navigate(page, 'cron');
  await appRequest(page, 'GET', '/api/v1/cron');
  expect((await snapshot(page)).currentPage).toBe('cron');
  const command = `echo e2e-ui-${Date.now()}`;
  const created = await api(page, 'POST', '/api/v1/cron', {
    expression: '*/5 * * * *',
    command,
    user: null,
    site_id: null,
  });
  expect(created.status).toBe(200);
  await navigate(page, 'cron');
  await expect
    .poll(async () => (await snapshot(page)).cronCommands, { timeout: 15_000 })
    .toContain(command);
});

test('backups page runs a backup through the API the UI uses', async ({ page }) => {
  await openApp(page);
  await expectSession(page);

  const before = await api(page, 'GET', '/api/v1/backups');
  expect(before.status).toBe(200);
  const run = await api(page, 'POST', '/api/v1/backups/run', { kind: 'full', site_id: null });
  expect(run.status).toBe(200);
  expect(run.body.name).toMatch(/^synthpanel-/);

  const after = await api(page, 'GET', '/api/v1/backups');
  expect(after.status).toBe(200);
  expect(after.body.backups.map((backup: any) => backup.id)).toContain(run.body.id);
});

test('backups page runs a backup through the UI', async ({ page }) => {
  await openApp(page);
  await skipWithoutHooks(
    page,
    ['navigate', 'snapshot'],
    'The backup button and result list are Slint-only; no DOM node can be clicked.',
  );
  await navigate(page, 'backups');
  await appRequest(page, 'GET', '/api/v1/backups');
  expect((await snapshot(page)).currentPage).toBe('backups');
  const run = await api(page, 'POST', '/api/v1/backups/run', { kind: 'full', site_id: null });
  expect(run.status).toBe(200);
  await navigate(page, 'backups');
  await expect
    .poll(async () => (await snapshot(page)).backups, { timeout: 15_000 })
    .toContain(run.body.name);
});

test('alerts page creates a channel and a rule through the API the UI uses', async ({ page }) => {
  await openApp(page);
  await expectSession(page);

  const name = `e2e-webhook-${Date.now()}`;
  const ruleName = `${name}-rule`;
  const channel = await api(page, 'POST', '/api/v1/alerts/channels', {
    name,
    kind: 'webhook',
    config: { url: 'https://example.com/hook' },
  });
  expect(channel.status).toBe(200);
  const channelId = channel.body.id;
  expect(channelId).toBeTruthy();

  const channels = await api(page, 'GET', '/api/v1/alerts/channels');
  expect(channels.status).toBe(200);
  expect(channels.body.channels.map((item: any) => item.name)).toContain(name);

  const rule = await api(page, 'POST', '/api/v1/alerts/rules', {
    name: ruleName,
    event: 'cpu.usage',
    severity: 'warning',
    channel_id: channelId,
    cooldown_secs: 900,
  });
  expect(rule.status).toBe(200);
  const rules = await api(page, 'GET', '/api/v1/alerts/rules');
  expect(rules.status).toBe(200);
  expect(rules.body.rules.map((item: any) => item.name)).toContain(ruleName);

  expect((await api(page, 'DELETE', `/api/v1/alerts/rules/${rule.body.id}`)).status).toBe(200);
  expect((await api(page, 'DELETE', `/api/v1/alerts/channels/${channelId}`)).status).toBe(200);
});

test('alerts page creates a channel and a rule through the UI', async ({ page }) => {
  await openApp(page);
  await skipWithoutHooks(
    page,
    ['navigate', 'snapshot'],
    'The alert forms, channels and rules are Slint models, not DOM elements.',
  );
  await navigate(page, 'alerts');
  await appRequest(page, 'GET', '/api/v1/alerts/channels');
  expect((await snapshot(page)).currentPage).toBe('alerts');
  const name = `e2e-ui-${Date.now()}`;
  const channel = await api(page, 'POST', '/api/v1/alerts/channels', {
    name,
    kind: 'webhook',
    config: { url: 'https://example.com/hook' },
  });
  expect(channel.status).toBe(200);
  await navigate(page, 'alerts');
  await expect
    .poll(async () => (await snapshot(page)).alertChannels, { timeout: 15_000 })
    .toContain(name);
});

test('deployments page exposes the deployment form data', async ({ page }) => {
  await openApp(page);
  await expectSession(page);

  const listed = await api(page, 'GET', '/api/v1/deployments');
  expect(listed.status).toBe(200);
  expect(Array.isArray(listed.body.deployments)).toBe(true);

  const createdSite = await api(page, 'POST', '/api/v1/sites', {
    domain: `e2e-deploy-${Date.now()}.example.com`,
    aliases: [],
    template: 'static',
    php_version: null,
    ssl_mode: 'none',
    document_root: '',
  });
  expect(createdSite.status).toBe(200);
  const siteId = createdSite.body.site.id;
  const repo = 'https://example.com/e2e-repo.git';
  const deployment = await api(page, 'POST', '/api/v1/deployments', {
    site_id: siteId,
    repo_url: repo,
    branch: 'main',
    target_subdir: null,
    build_command: null,
    deploy_user: 'www-data',
  });
  expect(deployment.status).toBe(200);
  const after = await api(page, 'GET', '/api/v1/deployments');
  expect(after.body.deployments.map((item: any) => item.repo_url)).toContain(repo);

  expect((await api(page, 'DELETE', `/api/v1/deployments/${deployment.body.id}`)).status).toBe(200);
  expect((await api(page, 'DELETE', `/api/v1/sites/${siteId}`)).status).toBe(200);
});

// The add form's `show-form` flag is private inside DeploymentsPage, so even a
// snapshot cannot see it. This test verifies the page and its data loaded;
// asserting the form itself needs a new exported Slint property (see report).
test('deployments page loads its model', async ({ page }) => {
  await openApp(page);
  await skipWithoutHooks(
    page,
    ['navigate', 'snapshot'],
    'The deployments page and its model are only observable through app state.',
  );
  await navigate(page, 'deployments');
  await appRequest(page, 'GET', '/api/v1/deployments');
  const state = await snapshot(page);
  expect(state.currentPage).toBe('deployments');
  expect(Array.isArray(state.deployments)).toBe(true);
});

test('php page lists the versions reported by the core', async ({ page }) => {
  await openApp(page);
  await expectSession(page);
  const versions = await api(page, 'GET', '/api/v1/php/versions');
  expect(versions.status).toBe(200);
  expect(Array.isArray(versions.body.versions)).toBe(true);
  const status = await api(page, 'GET', '/api/v1/php/status');
  expect(status.status).toBe(200);
  expect(status.body).toBeTruthy();
});

test('php page renders the versions', async ({ page }) => {
  await openApp(page);
  await skipWithoutHooks(
    page,
    ['navigate', 'snapshot'],
    'PHP version rows are drawn on the canvas and need a state snapshot.',
  );
  await navigate(page, 'php');
  await appRequest(page, 'GET', '/api/v1/php/status');
  expect(Array.isArray((await snapshot(page)).phpVersions)).toBe(true);
});

test('logs page reads a log source tail', async ({ page }) => {
  await openApp(page);
  await expectSession(page);
  const sources = await api(page, 'GET', '/api/v1/logs/sources');
  expect(sources.status).toBe(200);
  expect(Array.isArray(sources.body.sources)).toBe(true);
  const tail = await api(page, 'GET', '/api/v1/logs?type=panel&limit=50');
  expect(tail.status).toBe(200);
});

test('logs page renders the tail', async ({ page }) => {
  await openApp(page);
  await skipWithoutHooks(
    page,
    ['navigate', 'snapshot'],
    'Log lines are rendered in a Slint model, so a state snapshot is required.',
  );
  await navigate(page, 'logs');
  await appRequest(page, 'GET', '/api/v1/logs/sources');
  expect(Array.isArray((await snapshot(page)).logLines)).toBe(true);
});

// Test-owned id range; the Rust bridge uses 1 (dashboard) and 2 (terminal).
const TEST_SOCKET_ID = 900;

test('terminal page opens a session and its WebSocket', async ({ page }) => {
  await openApp(page);
  await expectSession(page);

  const opened = await api(page, 'POST', '/api/v1/terminal/open', { rows: 24, cols: 80 });
  expect(opened.status).toBe(200);
  const sessionId = opened.body.id;
  expect(sessionId).toBeTruthy();
  expect(opened.body.shell).toBeTruthy();

  await page.evaluate(
    ({ id, path }) => window.synthpanel_e2e!.openSocket(id, path),
    { id: TEST_SOCKET_ID, path: `/ws/terminal/${sessionId}` },
  );
  await page.evaluate((id) => window.synthpanel_e2e!.waitForSocket(id, 1, 15_000), TEST_SOCKET_ID);
  expect(await page.evaluate((id) => window.synthpanel_e2e!.socketState(id), TEST_SOCKET_ID)).toBe(1);

  await page.evaluate((id) => window.synthpanel_e2e!.closeSocket(id), TEST_SOCKET_ID);
  expect((await api(page, 'DELETE', `/api/v1/terminal/${sessionId}`)).status).toBe(200);
});

test('terminal page opens a shell through the UI', async ({ page }) => {
  await openApp(page);
  await skipWithoutHooks(
    page,
    ['navigate', 'snapshot'],
    'The terminal widget and its connection status are not exposed as DOM.',
  );
  await navigate(page, 'terminal');
  await appRequest(page, 'POST', '/api/v1/terminal/open');
  await expect
    .poll(async () => (await snapshot(page)).terminalStatus, { timeout: 20_000 })
    .toBe('');
  const state = await snapshot(page);
  expect(state.terminalId).toBeTruthy();
  expect(state.currentPage).toBe('terminal');
});

// Guard against forgetting the activation flag when adding new navigations.
test('e2e harness is inert on a normal URL', async ({ page }) => {
  await page.goto('/');
  expect(
    await page.evaluate(() => ({
      shim: typeof window.synthpanel_e2e,
      bridge: typeof window.synthpanel_api?.request,
    })),
  ).toMatchObject({ shim: 'undefined', bridge: 'function' });
});
