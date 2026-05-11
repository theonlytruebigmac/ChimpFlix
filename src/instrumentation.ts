// Next.js instrumentation hook. The `register()` export runs exactly once
// per Node process, before any request is served. We use it to prime the
// Plex data cache so a container restart doesn't pay a "first request
// cold" penalty.

export async function register() {
  console.log(`[instrument] register() called runtime=${process.env.NEXT_RUNTIME}`);
  if (process.env.NEXT_RUNTIME !== "nodejs") return;

  const [{ readBootstrapAuth }, { ensureWarmerStarted }] = await Promise.all([
    import("./lib/admin-token-store"),
    import("./lib/cache-warmer"),
  ]);

  const auth = await readBootstrapAuth();
  console.log(`[instrument] readBootstrapAuth → ${auth ? "found" : "missing"}`);
  if (auth) {
    // Don't await — the warmer fills the cache asynchronously while the
    // server starts accepting requests.
    ensureWarmerStarted(auth);
    console.log(`[instrument] ensureWarmerStarted called for server ${auth.id}`);
  }
}
