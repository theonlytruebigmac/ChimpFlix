"use client";

import { useEffect } from "react";

/**
 * Registers the service worker. Mounted once at the app root via the
 * RootLayout. The actual caching strategy lives in /public/sw.js.
 *
 * On dev (`next dev`), service workers can interfere with HMR — we skip
 * registration in that case and unregister any leftover SW from prior
 * production runs on the same origin.
 */
export function ServiceWorker() {
  useEffect(() => {
    if (typeof window === "undefined") return;
    if (!("serviceWorker" in navigator)) return;

    if (process.env.NODE_ENV !== "production") {
      // Tear down any previously-registered SW so dev reloads aren't
      // intercepted by stale cache entries.
      navigator.serviceWorker
        .getRegistrations()
        .then((regs) => regs.forEach((r) => r.unregister()))
        .catch(() => {});
      return;
    }

    const onLoad = () => {
      navigator.serviceWorker
        .register("/sw.js", { scope: "/" })
        .then((reg) => {
          // When a new SW is found, ask it to take over immediately so the
          // user gets the new code without waiting for all tabs to close.
          reg.addEventListener("updatefound", () => {
            const installing = reg.installing;
            if (!installing) return;
            installing.addEventListener("statechange", () => {
              if (
                installing.state === "installed" &&
                navigator.serviceWorker.controller
              ) {
                installing.postMessage?.({ type: "SKIP_WAITING" });
              }
            });
          });
        })
        .catch(() => {
          // SW registration failures are non-fatal — the app still works,
          // just without the speedups.
        });
    };

    if (document.readyState === "complete") onLoad();
    else window.addEventListener("load", onLoad, { once: true });
  }, []);

  return null;
}
