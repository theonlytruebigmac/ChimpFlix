"use client";

import { useEffect } from "react";
import { prefetchModalData } from "@/lib/modal-cache";

/**
 * Warms the modal-data cache for the first few items of a rail in browser
 * idle time. Cards already prefetch on hover, but the most-likely-clicked
 * ones (top of each rail) deserve to be ready before the user touches the
 * mouse. The 2-item cap is a compromise between snappiness and not flooding
 * Plex with concurrent requests when a page has many rails.
 */
export function RailPrefetch({ ratingKeys }: { ratingKeys: string[] }) {
  useEffect(() => {
    if (typeof window === "undefined") return;
    const ric =
      (window as Window & {
        requestIdleCallback?: (cb: () => void) => number;
      }).requestIdleCallback ?? ((cb: () => void) => window.setTimeout(cb, 0));
    const cic =
      (window as Window & {
        cancelIdleCallback?: (handle: number) => void;
      }).cancelIdleCallback ?? window.clearTimeout;
    const handle = ric(() => {
      for (const key of ratingKeys) prefetchModalData(key);
    });
    return () => {
      cic(handle);
    };
  }, [ratingKeys]);
  return null;
}
