<script lang="ts">
  import { onMount } from 'svelte';
  import { GitBranch, Play, Plus, Trash2 } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { formatDateTime } from '../lib/format';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import StatusBadge from '../lib/components/StatusBadge.svelte';

  interface Deployment {
    id: string;
    site_id: string;
    repo_url: string;
    branch: string;
    target_subdir: string;
    build_command: string;
    deploy_user: string;
    enabled: boolean;
    created_at: string;
  }

  interface DeployRun {
    id: string;
    commit_sha: string;
    branch: string;
    status: string;
    message: string;
    output: string;
    started_at: string;
    finished_at: string | null;
  }

  interface SiteRow {
    id: string;
    domain: string;
  }

  let deployments = $state<Deployment[]>([]);
  let sites = $state<SiteRow[]>([]);
  let runs = $state<Record<string, DeployRun[]>>({});
  let expanded = $state<string | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let status = $state('');
  let showForm = $state(false);

  let siteId = $state('');
  let repoUrl = $state('');
  let branch = $state('main');
  let subdir = $state('');
  let buildCommand = $state('');
  let deployUser = $state('www-data');

  async function load() {
    try {
      const [deploymentResult, siteResult] = await Promise.all([
        api<{ deployments: Deployment[] }>('/api/v1/deployments'),
        api<SiteRow[]>('/api/v1/sites'),
      ]);
      deployments = deploymentResult.deployments ?? [];
      sites = siteResult ?? [];
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load the deployments';
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function createDeployment() {
    busy = true;
    error = '';
    status = '';
    try {
      await api('/api/v1/deployments', {
        method: 'POST',
        body: JSON.stringify({
          site_id: siteId,
          repo_url: repoUrl,
          branch,
          target_subdir: subdir,
          build_command: buildCommand,
          deploy_user: deployUser,
        }),
      });
      status = 'The deployment was created.';
      repoUrl = '';
      buildCommand = '';
      showForm = false;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to create the deployment';
    } finally {
      busy = false;
    }
  }

  async function runDeployment(deployment: Deployment) {
    error = '';
    status = '';
    busy = true;
    try {
      const run = await api<DeployRun>(`/api/v1/deployments/${deployment.id}/run`, {
        method: 'POST',
      });
      status =
        run.status === 'ok'
          ? `Deployed ${run.commit_sha.slice(0, 8)}.`
          : `The deployment failed: ${run.message}`;
      if (expanded === deployment.id) await loadRuns(deployment.id);
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to run the deployment';
    } finally {
      busy = false;
    }
  }

  async function loadRuns(id: string) {
    try {
      const result = await api<{ runs: DeployRun[] }>(`/api/v1/deployments/${id}/runs`);
      runs = { ...runs, [id]: result.runs ?? [] };
    } catch {
      runs = { ...runs, [id]: [] };
    }
  }

  async function toggleRuns(deployment: Deployment) {
    if (expanded === deployment.id) {
      expanded = null;
      return;
    }
    expanded = deployment.id;
    await loadRuns(deployment.id);
  }

  async function removeDeployment(deployment: Deployment) {
    if (!confirm(`Delete the deployment for ${deployment.repo_url}?`)) return;
    try {
      await api(`/api/v1/deployments/${deployment.id}`, { method: 'DELETE' });
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to delete the deployment';
    }
  }

  function siteDomain(id: string): string {
    return sites.find((site) => site.id === id)?.domain ?? id;
  }
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Deployments</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        Pull a Git repository into a site, build it and publish it — with an
        automatic rollback when a step fails.
      </p>
    </div>
    <button class="btn-primary" onclick={() => (showForm = !showForm)}>
      <Plus size={15} />
      {showForm ? 'Cancel' : 'Add deployment'}
    </button>
  </div>

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">{error}</p>
  {/if}
  {#if status}
    <p class="rounded-lg bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">{status}</p>
  {/if}

  {#if showForm}
    <Card title="New deployment">
      <form
        class="space-y-4"
        onsubmit={(event) => {
          event.preventDefault();
          createDeployment();
        }}
      >
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label class="label" for="deploy-site">Site</label>
            <select id="deploy-site" class="input" bind:value={siteId} required>
              <option value="" disabled>Choose a site</option>
              {#each sites as site (site.id)}
                <option value={site.id}>{site.domain}</option>
              {/each}
            </select>
          </div>
          <div>
            <label class="label" for="deploy-branch">Branch</label>
            <input id="deploy-branch" class="input" bind:value={branch} />
          </div>
        </div>
        <div>
          <label class="label" for="deploy-repo">Repository</label>
          <input id="deploy-repo" class="input font-mono" bind:value={repoUrl} placeholder="https://github.com/example/app.git" required />
        </div>
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label class="label" for="deploy-subdir">Sub-directory</label>
            <input id="deploy-subdir" class="input" bind:value={subdir} placeholder="public" />
          </div>
          <div>
            <label class="label" for="deploy-user">Deploy user</label>
            <input id="deploy-user" class="input" bind:value={deployUser} />
          </div>
        </div>
        <div>
          <label class="label" for="deploy-build">Build command</label>
          <input id="deploy-build" class="input font-mono" bind:value={buildCommand} placeholder="npm ci && npm run build" />
        </div>
        <button class="btn-primary" type="submit" disabled={busy || !repoUrl.trim() || !siteId}>
          {busy ? 'Creating…' : 'Create deployment'}
        </button>
      </form>
    </Card>
  {/if}

  <Card title="Deployments" subtitle="{deployments.length} deployment(s)">
    {#if loading}
      <p class="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
    {:else if deployments.length === 0}
      <EmptyState title="No deployments yet" description="Connect a repository to publish your code.">
        {#snippet icon()}<GitBranch size={28} />{/snippet}
      </EmptyState>
    {:else}
      <ul class="divide-y divide-slate-100 dark:divide-slate-800">
        {#each deployments as deployment (deployment.id)}
          <li class="py-3">
            <div class="flex flex-wrap items-center justify-between gap-3">
              <div class="min-w-0">
                <p class="truncate font-mono text-sm">{deployment.repo_url}</p>
                <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">
                  {siteDomain(deployment.site_id)} · branch {deployment.branch}
                  {#if deployment.target_subdir}· sub-directory {deployment.target_subdir}{/if}
                  {#if deployment.build_command}· build <span class="font-mono">{deployment.build_command}</span>{/if}
                </p>
              </div>
              <div class="flex items-center gap-2">
                <button class="btn-secondary px-2.5" title="Deploy now" aria-label="Deploy {deployment.repo_url}" onclick={() => runDeployment(deployment)} disabled={busy}>
                  <Play size={15} />
                </button>
                <button class="btn-secondary px-2.5" onclick={() => toggleRuns(deployment)}>
                  {expanded === deployment.id ? 'Hide runs' : 'Runs'}
                </button>
                <button class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400" title="Delete" aria-label="Delete {deployment.repo_url}" onclick={() => removeDeployment(deployment)}>
                  <Trash2 size={15} />
                </button>
              </div>
            </div>
            {#if expanded === deployment.id}
              <div class="mt-3 rounded-lg bg-slate-50 p-3 dark:bg-slate-950">
                {#if (runs[deployment.id] ?? []).length === 0}
                  <p class="text-xs text-slate-500 dark:text-slate-400">No runs recorded yet.</p>
                {:else}
                  <ul class="space-y-2">
                    {#each runs[deployment.id] as run (run.id)}
                      <li class="text-xs">
                        <div class="flex flex-wrap items-center gap-2">
                          <StatusBadge status={run.status === 'ok' ? 'ok' : 'failed'} />
                          <span class="font-mono">{run.commit_sha.slice(0, 8)}</span>
                          <span class="text-slate-500 dark:text-slate-400">
                            {formatDateTime(run.started_at)}
                          </span>
                        </div>
                        {#if run.message && run.status !== 'ok'}
                          <p class="mt-1 text-rose-600 dark:text-rose-400">{run.message}</p>
                        {/if}
                        {#if run.output}
                          <pre class="mt-1 max-h-40 overflow-auto rounded bg-white p-2 font-mono dark:bg-slate-900">{run.output}</pre>
                        {/if}
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>
