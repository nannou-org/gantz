// Service Worker for caching PWA

const CACHE_NAME = 'gantz-cache-v1';
const urlsToCache = [
    '/',
    '/index.html',
    '/gantz.d.ts',
    '/gantz.js',
    '/gantz_bg.wasm',
    '/gantz_bg.wasm.d.ts',
    '/manifest.json',
    '/sw.js',
];

self.addEventListener('install', event => {
    event.waitUntil(
        caches.open(CACHE_NAME)
            .then(cache => cache.addAll(urlsToCache))
    );
});

self.addEventListener('fetch', event => {
    event.respondWith(
        caches.match(event.request)
            .then(response => response || fetch(event.request))
    );
});
