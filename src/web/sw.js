const CACHE_NAME = '__BEATLAB_SW_VERSION__';

const PRECACHE_URLS = [
  '/',
  '/manifest.webmanifest',
  '/web/favicon.svg',
  '/web/icon-192.png',
  '/web/icon-512.png',
  '/web/icon-maskable-512.png',
  '/web/apple-touch-icon.png',
];

const STATIC_PATHS = new Set([
  '/manifest.webmanifest',
  '/web/manifest.webmanifest',
  '/favicon.svg',
  '/web/favicon.svg',
  '/web/icon-192.png',
  '/web/icon-512.png',
  '/web/icon-maskable-512.png',
  '/web/apple-touch-icon.png',
  '/apple-touch-icon.png',
  '/web/styles.css',
  '/web/app.js',
]);

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) =>
        cache.addAll(PRECACHE_URLS.map((url) => new Request(url, { cache: 'reload' })))
      )
      .then(() => self.skipWaiting())
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys
            .filter((key) => key.startsWith('beatlab-pwa-') && key !== CACHE_NAME)
            .map((key) => caches.delete(key))
        )
      )
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;

  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;

  // Leave API and range-based media stream requests untouched.
  if (url.pathname.startsWith('/api/')) {
    return;
  }

  // App shell navigations: network-first so updates land immediately, with offline cache fallback.
  if (req.mode === 'navigate' && (url.pathname === '/' || url.pathname === '/web')) {
    event.respondWith(
      fetch(req)
        .then((resp) => {
          if (resp && resp.ok && resp.type === 'basic') {
            const copy = resp.clone();
            caches.open(CACHE_NAME).then((cache) => cache.put('/', copy)).catch(() => {});
          }
          return resp;
        })
        .catch(() =>
          caches.open(CACHE_NAME).then((cache) =>
            cache.match('/').then(
              (cached) =>
                cached ||
                new Response(
                  '<!doctype html><html><body style="background:#0c0e13;color:#f3f5f8;font-family:system-ui,sans-serif;display:grid;place-items:center;height:100vh;margin:0"><div style="text-align:center;padding:24px"><h2>Beatlab is offline</h2><p style="color:#98a2b3">Check your connection and try again.</p></div></body></html>',
                  { status: 503, headers: { 'Content-Type': 'text/html; charset=utf-8' } }
                )
            )
          )
        )
    );
    return;
  }

  // Static PWA assets: serve from versioned cache with network fallback.
  if (STATIC_PATHS.has(url.pathname)) {
    event.respondWith(
      caches.open(CACHE_NAME).then((cache) =>
        cache.match(req, { ignoreSearch: url.pathname === '/web/favicon.svg' }).then((cached) => {
          if (cached) return cached;
          return fetch(req).then((resp) => {
            if (resp && resp.ok && resp.type === 'basic') {
              cache.put(req, resp.clone()).catch(() => {});
            }
            return resp;
          });
        })
      )
    );
  }
});
