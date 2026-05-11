// Pre-warming for the watch flow. Triggered on Play-button hover to mask
// the chunk-loading + transcoder startup latency that otherwise happens
// after the user actually clicks.
//
// What we pre-warm:
//   1. The hls.js chunk — it's a dynamic import inside Player, so the
//      browser doesn't fetch it until <Player> mounts. Doing it on hover
//      means the chunk is in module cache by click time.
//   2. Nothing else here today; the watch route's RSC payload gets
//      prefetched separately via router.prefetch on Link hover.
//
// Idempotent and safe to call repeatedly — `import()` returns a cached
// Promise after the first call.

let started = false;

export function prefetchPlay(): void {
  if (started || typeof window === "undefined") return;
  started = true;
  // Fire-and-forget. If the chunk fails to load now, the regular dynamic
  // import inside Player will retry on mount.
  import("hls.js").catch(() => {
    started = false;
  });
}
