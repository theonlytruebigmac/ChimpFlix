"use client";

import { useEffect } from "react";

// One-time cleanup: the app used to ship a caching service worker; we
// removed it. This component runs on every page load and unregisters any
// surviving SW + drops any caches it owned. Cheap when nothing is
// registered (a single getRegistrations() returning []).
//
// Safe to delete the import + this file once you're confident no clients
// are still running the old worker.
export function ServiceWorker() {
  useEffect(() => {
    if (typeof window === "undefined") return;
    if (!("serviceWorker" in navigator)) return;

    navigator.serviceWorker
      .getRegistrations()
      .then((regs) => Promise.all(regs.map((r) => r.unregister())))
      .catch(() => {});

    if ("caches" in window) {
      // Only delete caches owned by this app's old service worker (prefixed
      // "cf-") so we don't wipe caches created by other libraries or the
      // browser under the same origin.
      caches
        .keys()
        .then((names) =>
          Promise.all(
            names.filter((n) => n.startsWith("cf-")).map((n) => caches.delete(n))
          )
        )
        .catch(() => {});
    }
  }, []);

  return null;
}
