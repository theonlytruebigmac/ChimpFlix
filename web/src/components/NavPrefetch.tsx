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
    PREFETCH_ROUTES.forEach((route, i) => {
      // Stagger the kicks so we don't queue 5 prefetch requests in the same
      // idle slice — gives the browser room to keep first paint snappy.
      const h = ric(() => {
        try {
          router.prefetch(route);
        } catch {
          // ignore — prefetch is best-effort
        }
        // Schedule the next route on a fresh idle callback.
        if (i < PREFETCH_ROUTES.length - 1) {
          handles.push(ric(() => router.prefetch(PREFETCH_ROUTES[i + 1])));
        }
      });
      handles.push(h);
    });

    return () => {
      handles.forEach((h) => cic(h));
    };
  }, [router]);

  return null;
}
