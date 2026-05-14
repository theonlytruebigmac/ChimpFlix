// Play-URL prefetcher. The old Plex flow warmed the transcode start URL
// on hover to shave the cold-click delay. The Rust stream session is
// created server-side at /watch render time, so there's nothing left to
// prefetch from the browser — this hook is a no-op kept so the existing
// `onMouseEnter={prefetchPlay}` call sites don't have to be touched.
//
// When we want hover prefetching back, the right move is `router.prefetch`
// on the /watch route, which Next handles efficiently.
export function prefetchPlay(): void {
  // intentional no-op
}
