// SynthPanel Slint bootstrap.
//
// The WASM module owns the UI. This shim only provides the browser primitives
// the Slint side cannot reach directly: same-origin fetch with cookies, the
// CSRF header, WebSockets and localStorage.

const API_BASE = '';

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
  const headers = {};
  if (body !== undefined && body !== null) {
    headers['content-type'] = 'application/json';
  }
  const verb = (method || 'GET').toUpperCase();
  if (verb !== 'GET' && verb !== 'HEAD') {
    const csrf = readCookie('synthpanel_csrf');
    if (csrf) headers['x-csrf-token'] = csrf;
  }

  const response = await fetch(`${API_BASE}${path}`, {
    method: verb,
    headers,
    credentials: 'same-origin',
    body: body === undefined || body === null ? undefined : body,
  });

  const text = await response.text();
  let payload = null;
  if (text.length > 0) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = text;
    }
  }
  return JSON.stringify({ status: response.status, body: payload });
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

window.synthpanel_api = {
  request,
  socket_open: socketOpen,
  socket_send: socketSend,
  socket_close: socketClose,
  storage_get: (key) => window.localStorage.getItem(key),
  storage_set: (key, value) => window.localStorage.setItem(key, value),
  apply_theme: applyTheme,
};

async function boot() {
  try {
    const module = await import('./pkg/synthpanel_ui.js');
    await module.default();
  } catch (error) {
    console.error('SynthPanel failed to start', error);
    const box = document.getElementById('boot-error');
    if (box) {
      box.style.display = 'grid';
      box.textContent = `SynthPanel could not start.\n\n${error && error.message ? error.message : error}`;
    }
  }
}

boot();
