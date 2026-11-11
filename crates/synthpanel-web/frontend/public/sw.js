/*
 * SynthPanel service worker.
 *
 * The worker caches the application shell so the panel opens instantly and
 * stays usable when the server is briefly unreachable. API requests are never
 * cached: stale metrics or stale configuration would be worse than an error.
 */

const CACHE = 'synthpanel-shell-v1';
const SHELL = ['/', '/index.html', '/manifest.webmanifest', '/icon-192.png', '/icon-512.png'];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.addAll(SHELL))
      .then(() => self.skipWaiting())
      .catch(() => self.skipWaiting()),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((key) => key !== CACHE).map((key) => caches.delete(key))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET') return;

  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;
  // Never cache the API or the WebSocket upgrade.
  if (url.pathname.startsWith('/api/') || url.pathname.startsWith('/ws/')) return;

  // Hashed assets are immutable: serve them from the cache first.
  if (url.pathname.startsWith('/assets/')) {
    event.respondWith(
      caches.match(request).then((cached) => {
        if (cached) return cached;
        return fetch(request).then((response) => {
          if (response.ok) {
            const copy = response.clone();
            caches.open(CACHE).then((cache) => cache.put(request, copy));
          }
          return response;
        });
      }),
    );
    return;
  }

  // Everything else: network first, fall back to the cached shell.
  event.respondWith(
    fetch(request)
      .then((response) => {
        if (response.ok && (request.mode === 'navigate' || url.pathname === '/')) {
          const copy = response.clone();
          caches.open(CACHE).then((cache) => cache.put('/index.html', copy));
        }
        return response;
      })
      .catch(() => caches.match('/index.html').then((cached) => cached || caches.match('/'))),
  );
});
