// SynthPanel Slint bootstrap.
//
// The WASM module owns the UI. This shim only provides the browser primitives
// the Slint side cannot reach directly: same-origin fetch with cookies, the
// CSRF header, WebSockets and localStorage.
//
// Test mode: loading the page with `?e2e=1` additionally installs
// `window.synthpanel_e2e`, a small harness for Playwright. Slint renders to a
// single <canvas> and exposes no semantic DOM, so the harness observes the
// bridge (requests, sockets, boot state) and exposes stable entry points for
// the Rust side to drive/query the UI. On a normal URL nothing is installed
// and the bridge behaves exactly as before.

const API_BASE = '';

const E2E_MODE = new URLSearchParams(window.location.search).get('e2e') === '1';
const MAX_REQUEST_LOG = 500;
const requestLog = [];
let bootState = 'idle'; // idle | loading | running | error
let bootError = null;
let readyHookCalled = false;

function logRequest(entry) {
  if (!E2E_MODE) return;
  requestLog.push(entry);
  if (requestLog.length > MAX_REQUEST_LOG) requestLog.shift();
}

// Rust passes an already-serialised JSON string; tests pass objects.
function normaliseBody(body) {
  if (body === undefined || body === null) return null;
  return typeof body === 'string' ? body : JSON.stringify(body);
}

function readCookie(name) {
  const prefix = `${name}=`;
  const entry = document.cookie
    .split(';')
    .map((part) => part.trim())
    .find((part) => part.startsWith(prefix));
  return entry ? decodeURIComponent(entry.slice(prefix.length)) : null;
}

/**
 * Request(method, path, body) -> Promise<string>
 *
 * Resolves to JSON `{status, body}`; the Rust side parses it. Network errors
 * reject the promise.
 */
async function request(method, path, body) {
  const verb = (method || 'GET').toUpperCase();
  const started = Date.now();
  let status = 0;
  try {
    const headers = {};
    const payload = normaliseBody(body);
    if (payload !== null) headers['content-type'] = 'application/json';
    if (verb !== 'GET' && verb !== 'HEAD') {
      const csrf = readCookie('synthpanel_csrf');
      if (csrf) headers['x-csrf-token'] = csrf;
    }

    const response = await fetch(`${API_BASE}${path}`, {
      method: verb,
      headers,
      credentials: 'same-origin',
      body: payload,
    });

    status = response.status;
    const text = await response.text();
    let parsed = null;
    if (text.length > 0) {
      try {
        parsed = JSON.parse(text);
      } catch {
        parsed = text;
      }
    }
    return JSON.stringify({ status, body: parsed });
  } finally {
    logRequest({ method: verb, path: String(path), status, at: started, durationMs: Date.now() - started });
  }
}

const sockets = new Map();

function socketOpen(id, path) {
  socketClose(id);
  const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
  const socket = new WebSocket(`${scheme}://${window.location.host}${path}`);
  sockets.set(id, socket);
  socket.onmessage = (event) => {
    if (typeof window.synthpanel_socket_message === 'function') {
      window.synthpanel_socket_message([id, String(event.data)]);
    }
  };
  socket.onclose = () => {
    sockets.delete(id);
    if (typeof window.synthpanel_socket_close === 'function') {
      window.synthpanel_socket_close(id);
    }
  };
  socket.onerror = () => socket.close();
}

function socketSend(id, data) {
  const socket = sockets.get(id);
  if (socket && socket.readyState === WebSocket.OPEN) socket.send(data);
}

function socketClose(id) {
  const socket = sockets.get(id);
  if (socket) {
    sockets.delete(id);
    socket.close();
  }
}

function applyTheme(theme) {
  document.documentElement.classList.toggle('dark', theme === 'dark');
  document.documentElement.style.colorScheme = theme;
}

/** Upload a file. Empty base64 opens a picker; resolves when done/cancelled. */
function uploadFile(scope, path, base64) {
  if (base64) {
    return request('POST', '/api/v1/files/upload',
      JSON.stringify({ scope, path, content_base64: base64 })).then(() => undefined);
  }
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.style.display = 'none';
    document.body.appendChild(input);
    input.addEventListener('change', async () => {
      try {
        const file = input.files && input.files[0];
        if (file) {
          if (file.size > 8 * 1024 * 1024) throw new Error('file is larger than 8 MiB');
          const data = await new Promise((done, fail) => {
            const reader = new FileReader();
            reader.onload = () => done(String(reader.result).split(',')[1] || '');
            reader.onerror = () => fail(reader.error);
            reader.readAsDataURL(file);
          });
          const dir = path.replace(/\/$/, '');
          const target = `${dir}/${file.name}`;
          await request('POST', '/api/v1/files/upload',
            JSON.stringify({ scope, path: target, content_base64: data }));
        }
      } catch (error) {
        console.error('SynthPanel upload failed', error);
      } finally {
        input.remove();
        resolve(undefined);
      }
    });
    input.click();
  });
}

window.synthpanel_api = {
  request,
  socket_open: socketOpen,
  socket_send: socketSend,
  socket_close: socketClose,
  storage_get: (key) => window.localStorage.getItem(key),
  storage_set: (key, value) => window.localStorage.setItem(key, value),
  apply_theme: applyTheme,
  upload_file: uploadFile,
};

// ---------------------------------------------------------------------------
// E2E harness (?e2e=1 only)
// ---------------------------------------------------------------------------

/** Rust-side hooks, when present, are plain globals named synthpanel_e2e_*. */
function e2eHook(name) {
  const fn = window[`synthpanel_e2e_${name}`];
  return typeof fn === 'function' ? fn : null;
}

function hasHook(name) {
  return e2eHook(name) !== null;
}

function requireHook(name) {
  const fn = e2eHook(name);
  if (!fn) {
    throw new Error(
      `Missing Rust E2E hook window.synthpanel_e2e_${name}() — the Slint app must expose it for this test.`,
    );
  }
  return fn;
}

/** JSON-parsed version of the bridge response; never throws on HTTP errors. */
async function apiRequest(method, path, body) {
  return JSON.parse(await request(method, path, normaliseBody(body)));
}

/** Like apiRequest but rejects on a >=400 status. */
async function apiOk(method, path, body) {
  const { status, body: value } = await apiRequest(method, path, body);
  if (status >= 400) {
    throw new Error(`${String(method).toUpperCase()} ${path} -> ${status}: ${JSON.stringify(value)}`);
  }
  return value;
}

function rustCallbacksInstalled() {
  return (
    typeof window.synthpanel_socket_message === 'function' &&
    typeof window.synthpanel_socket_close === 'function'
  );
}

/**
 * Resolve once the Slint WASM side has installed its bridge callbacks.
 *
 * Rust can call `window.synthpanel_e2e_ready()` at the end of install() for a
 * deterministic signal; the fallback observes the socket callbacks that
 * app::install() registers before entering the event loop.
 */
async function waitForReady(timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (bootState === 'error') {
      throw new Error(`SynthPanel failed to boot: ${bootError}`);
    }
    if (readyHookCalled || rustCallbacksInstalled()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(
    `Timed out waiting for the Slint app (bootState=${bootState}, readyHook=${readyHookCalled}, callbacks=${rustCallbacksInstalled()})`,
  );
}

/** Wait for a request made *through the bridge* (i.e. by the Slint app). */
async function waitForRequest(method, path, timeoutMs = 10_000) {
  const verb = String(method || 'GET').toUpperCase();
  const needle = String(path || '');
  const matches = (entry) => entry.method === verb && entry.path.includes(needle);
  const found = requestLog.find(matches);
  if (found) return found;

  let cursor = requestLog.length;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    for (let i = cursor; i < requestLog.length; i += 1) {
      if (matches(requestLog[i])) return requestLog[i];
    }
    cursor = requestLog.length;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  const seen = requestLog.map((entry) => `${entry.method} ${entry.path} (${entry.status})`);
  throw new Error(`Timed out waiting for ${verb} ${needle}; bridge requests seen: ${JSON.stringify(seen)}`);
}

function openSocket(id, path) {
  socketOpen(Number(id), String(path));
}

function socketState(id) {
  const socket = sockets.get(Number(id));
  return socket ? socket.readyState : null;
}

function closeSocket(id) {
  socketClose(Number(id));
}

async function waitForSocket(id, readyState = 1, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (socketState(id) === readyState) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(`Timed out waiting for socket ${id} to reach readyState ${readyState} (now ${socketState(id)})`);
}

if (E2E_MODE) {
  // Owned by the shim: Rust calls this, it must not re-register the name.
  // Exposed on the bridge as well so Rust can use its existing api::call helper.
  const markReady = () => {
    readyHookCalled = true;
  };
  window.synthpanel_e2e_ready = markReady;
  window.synthpanel_api.e2e_ready = markReady;

  window.synthpanel_e2e = {
    enabled: true,
    bootState: () => bootState,
    hasHook,
    requestLog: () => requestLog.map((entry) => ({ ...entry })),
    clearRequestLog: () => {
      requestLog.length = 0;
    },
    apiRequest,
    apiOk,
    waitForReady,
    waitForRequest,
    // Semantic actions/state reads are delegated to optional Rust hooks.
    login: (email, password) =>
      requireHook('login')({ email: String(email), password: String(password) }),
    navigate: (page) => requireHook('navigate')(String(page)),
    snapshot: () => {
      const raw = requireHook('snapshot')();
      return typeof raw === 'string' ? JSON.parse(raw) : raw;
    },
    openSocket,
    socketState,
    waitForSocket,
    closeSocket,
    storageGet: (key) => window.localStorage.getItem(String(key)),
    storageSet: (key, value) => window.localStorage.setItem(String(key), String(value)),
  };
}

async function boot() {
  try {
    if (E2E_MODE) bootState = 'loading';
    const module = await import('./pkg/synthpanel_ui.js');
    // The generated init function runs the wasm start hook synchronously: it
    // registers the bridge callbacks, loads the session and enters the Slint
    // event loop. Setting this before the call lets waitForReady() observe a
    // failed import even though the loop itself never returns normally.
    if (E2E_MODE) bootState = 'running';
    try {
      await module.default();
    } catch (error) {
      const message = error && error.message ? error.message : String(error);
      if (message.includes('Using exceptions for control flow')) return;
      throw error;
    }
  } catch (error) {
    const message = error && error.message ? error.message : String(error);
    if (E2E_MODE) {
      bootState = 'error';
      bootError = message;
    }
    console.error('SynthPanel failed to start', error);
    const box = document.getElementById('boot-error');
    if (box) {
      box.style.display = 'grid';
      box.textContent = `SynthPanel could not start.\n\n${message}`;
    }
  }
}

boot();
