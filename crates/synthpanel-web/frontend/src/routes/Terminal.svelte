<script lang="ts">
  import { onMount } from 'svelte';
  import { Terminal as TerminalIcon } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import Card from '../lib/components/Card.svelte';

  interface Session {
    id: string;
    shell: string;
    cwd: string;
    created_at: string;
  }

  let host = $state<HTMLDivElement | null>(null);
  let session = $state<Session | null>(null);
  let error = $state('');
  let status = $state('');
  let busy = $state(false);
  let closed = $state(false);

  onMount(() => {
    let disposed = false;
    let socket: WebSocket | null = null;
    let term: import('@xterm/xterm').Terminal | null = null;
    let fit: (() => void) | null = null;

    async function start() {
      busy = true;
      error = '';
      try {
        const opened = await api<Session>('/api/v1/terminal/open', {
          method: 'POST',
          body: JSON.stringify({ rows: 24, cols: 80 }),
        });
        if (disposed) return;
        session = opened;

        // The terminal emulator is loaded on demand so the panel stays small.
        const [{ Terminal: XTerm }, css] = await Promise.all([
          import('@xterm/xterm'),
          import('@xterm/xterm/css/xterm.css'),
        ]);
        void css;
        if (disposed || !host) return;

        term = new XTerm({
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
          fontSize: 13,
          cursorBlink: true,
          convertEol: false,
          theme: {
            background: '#0b1220',
            foreground: '#d7dce5',
            cursor: '#60a5fa',
          },
        });
        term.open(host);
        term.focus();

        const { FitAddon } = await import('@xterm/addon-fit');
        const fitAddon = new FitAddon();
        term.loadAddon(fitAddon);
        fit = () => {
          try {
            fitAddon.fit();
          } catch {
            // The container may not be laid out yet.
          }
        };
        fit();
        window.addEventListener('resize', fit);

        const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
        socket = new WebSocket(`${scheme}://${window.location.host}/ws/terminal/${opened.id}`);
        socket.onopen = () => {
          status = `Connected to ${opened.shell}`;
          fit?.();
          const rows = term?.rows ?? 24;
          const cols = term?.cols ?? 80;
          socket?.send(JSON.stringify({ type: 'resize', rows, cols }));
        };
        socket.onmessage = (event) => {
          try {
            const frame = JSON.parse(event.data as string);
            if (frame.type === 'output') term?.write(frame.data as string);
            if (frame.type === 'closed') closed = true;
            if (frame.type === 'error') error = frame.message as string;
          } catch {
            // Ignore frames that are not JSON.
          }
        };
        socket.onclose = () => {
          status = 'The session ended.';
        };
        socket.onerror = () => {
          error = 'The terminal connection failed.';
        };

        term.onData((data) => {
          if (socket?.readyState === WebSocket.OPEN) {
            socket.send(JSON.stringify({ type: 'input', data }));
          }
        });
      } catch (err) {
        error = err instanceof ApiError ? err.message : 'Unable to open the terminal';
      } finally {
        busy = false;
      }
    }

    start();

    return () => {
      disposed = true;
      if (fit) window.removeEventListener('resize', fit);
      socket?.close();
      term?.dispose();
      if (session) {
        void api(`/api/v1/terminal/${session.id}`, { method: 'DELETE' }).catch(() => undefined);
      }
    };
  });
</script>

<div class="space-y-6">
  <div>
    <h1 class="text-2xl font-semibold">Terminal</h1>
    <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
      A root shell on this server. The session runs inside the privileged core
      and is closed when you leave the page.
    </p>
  </div>

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">{error}</p>
  {/if}
  {#if status}
    <p class="text-xs text-slate-500 dark:text-slate-400">{status}</p>
  {/if}

  <Card title="Shell" subtitle={session ? `${session.shell} in ${session.cwd}` : 'connecting…'}>
    {#if closed}
      <p class="mb-2 text-sm text-amber-600 dark:text-amber-400">
        The shell exited. Reload the page to open a new session.
      </p>
    {/if}
    <div
      bind:this={host}
      class="h-[28rem] overflow-hidden rounded-lg bg-[#0b1220] p-2"
      aria-label="Terminal"
    ></div>
    {#if busy}
      <p class="mt-2 text-sm text-slate-500 dark:text-slate-400">
        <span class="inline-flex items-center gap-2">
          <TerminalIcon size={14} /> Starting the shell…
        </span>
      </p>
    {/if}
  </Card>
</div>
