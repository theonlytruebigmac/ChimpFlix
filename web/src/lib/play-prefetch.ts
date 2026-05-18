// Play-button hover warmer.
//
// The full session pre-warm (start an HLS encode session on hover so
// the first segment is ready when the user clicks Play) is parked
// behind a backend refactor — the watch page's ratingKey-to-
// media_file_id resolver lives in the page component and would need
// to move to a shared library function before an `/api/v1/stream/
// prewarm` endpoint could call it. Until then, this hook delivers
// the cheap-but-real wins that don't need that refactor:
//
//   * `detectClientCapabilities()` — a `canPlayType` sweep that the
//     player runs on every session start. ~5-20ms per call, cached
//     after first run. Calling it on hover means the player's first
//     mount finds the cache populated.
//
//   * `getPrefs()` — a localStorage read + JSON parse. Sub-ms, but
//     populating the module-scoped cache up front means the player
//     can read prefs synchronously without a fresh disk hit.
//
// Route and server-rendered HTML prefetch is handled by Next's
// `<Link prefetch>` on the modal Play button itself (visible-link
// auto-prefetch + an explicit warm on this hover handler).
import { detectClientCapabilities } from "@/lib/client-caps";
import { getPrefs } from "@/lib/prefs";

export function prefetchPlay(): void {
  // Both calls are idempotent and module-cached after the first
  // invocation. Wrapping each in try/catch keeps a misbehaving
  // browser API (some content-blocked iframes throw on
  // canPlayType) from breaking the hover handler.
  try {
    detectClientCapabilities();
  } catch {
    // ignore
  }
  try {
    getPrefs();
  } catch {
    // ignore
  }
}
