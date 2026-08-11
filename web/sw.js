// Minimal service worker so Chrome/ChromeOS treat BearCAD as installable.
// Network-only fetch: do not cache app assets — the page versions them with a
// build stamp (#1049) and a SW cache would re-introduce mismatched glue/wasm pairs.
self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("fetch", (event) => {
  event.respondWith(fetch(event.request));
});
