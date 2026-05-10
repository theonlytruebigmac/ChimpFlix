// Next.js instrumentation hook. The `register()` export runs exactly once
// per Node process, before any request is served. We use it to prime the
// Plex data cache so a container restart doesn't pay a "first request
// cold" penalty.

export async function register() {
  if (process.env.NEXT_RUNTIME !== "nodejs") return;

  const [{ readBootstrapAuth }, { ensureWarmerStarted }] = await Promise.all([
    import("./lib/admin-token-store"),
    import("./lib/cache-warmer"),
  ]);

  const auth = await readBootstrapAuth();
  if (auth) {
    // Don't await — the warmer fills the cache asynchronously while the
    // server starts accepting requests.
    ensureWarmerStarted(auth);
  }
}
