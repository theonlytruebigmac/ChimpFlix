"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";

// Routes the user is most likely to visit after landing on any page. Worth
// pre-warming during browser idle time so click-to-paint feels instant.
// Library pages aren't included — they're dynamic and prefetching them all
// would over-fetch for users with many libraries.
const PREFETCH_ROUTES = ["/", "/new-popular", "/my-list"];

/**
 * Idle-time route prefetcher. Mounted once at the app root so every page
 * benefits. Uses requestIdleCallback so this work doesn't compete with
 * first paint or the initial data stream.
 */
export function NavPrefetch() {
  const router = useRouter();

  useEffect(() => {
    if (typeof window === "undefined") return;
    const ric =
      (window as Window & {
        requestIdleCallback?: (cb: () => void) => number;
      }).requestIdleCallback ??
      ((cb: () => void) => window.setTimeout(cb, 1500));
    const cic =
      (window as Window & {
        cancelIdleCallback?: (handle: number) => void;
      }).cancelIdleCallback ?? window.clearTimeout;

    const handles: number[] = [];
    // Each route gets its own idle callback so they don't all compete in one
    // idle slice — gives the browser room to keep first paint snappy.
    PREFETCH_ROUTES.forEach((route) => {
      handles.push(
        ric(() => {
          try {
            router.prefetch(route);
          } catch {
            // ignore — prefetch is best-effort
          }
        }),
      );
    });

    return () => {
      handles.forEach((h) => cic(h));
    };
  }, [router]);

  return null;
}
