// Tombstone service worker. The app previously shipped a caching SW; we
// removed it because Next's hashed-static caching + RSC soft-nav already
// covered the common cases and the cache-versioning dance was costly. This
// file stays so that any client whose old SW is still intercepting fetches
// updates to a worker that immediately uninstalls itself and clears the
// caches it owned. Once we're confident no clients are still running the
// old SW, this file (and its `<script>` registration in the layout) can be
// deleted outright.

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const names = await caches.keys();
      await Promise.all(names.map((n) => caches.delete(n)));
      await self.registration.unregister();
      const clientList = await self.clients.matchAll({ type: "window" });
      // Force a reload so the tab stops being controlled by this worker.
      for (const client of clientList) {
        client.navigate(client.url).catch(() => {});
      }
    })(),
  );
});
