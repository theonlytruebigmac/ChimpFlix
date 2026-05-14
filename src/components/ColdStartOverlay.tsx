"use client";

import { useEffect, useState } from "react";
import { brandNameUpper } from "@/lib/env";

/**
 * Full-screen "preparing your library" overlay shown when the cache
 * warmer hasn't completed its first cycle. Without this, the user lands
 * on the page during cold-start and sees skeletons for 30-60s while
 * Plex's expensive library queries (large movie libraries sorting on
 * viewCount, etc.) populate the cache — looks broken even though it's
 * working.
 *
 * Behavior:
 *   - SSR-rendered when `initiallyReady=false` so the user never sees
 *     a frame of skeletons during cold-start.
 *   - Client polls `/api/warmer-status` every 2s. As soon as the
 *     warmer reports ready, we full-reload so the user gets an SSR
 *     render against the warm cache (= every rail is a cache hit).
 *   - When `initiallyReady=true` the component returns null. Callers
 *     should still mount it — the warmer state can flip mid-session
 *     after a server / profile switch (we reset it in
 *     ensureWarmerStarted) and the polling will catch up.
 */
export function ColdStartOverlay({
  initiallyReady,
}: {
  initiallyReady: boolean;
}) {
  const [ready, setReady] = useState(initiallyReady);

  useEffect(() => {
    if (ready) return;
    let cancelled = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const r = await fetch("/api/warmer-status", { cache: "no-store" });
        if (cancelled) return;
        if (r.ok) {
          const data = (await r.json()) as { ready?: boolean };
          if (data.ready) {
            // Full reload (not router.refresh) so the page re-renders
            // server-side against the now-warm cache and we get a real
            // first-paint of content instead of seeing skeletons
            // resolve one by one.
            window.location.reload();
            return;
          }
        }
      } catch {
        // Network blip — keep polling.
      }
      if (!cancelled) {
        timer = window.setTimeout(poll, 2000);
      }
    };
    // Small initial delay so we don't fire immediately on mount and
    // race the server's own first-tick completion in the same tick.
    timer = window.setTimeout(poll, 500);
    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [ready]);

  if (ready) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      className="fixed inset-0 z-[100] flex flex-col items-center justify-center bg-black"
    >
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_top,rgba(70,12,16,0.45)_0%,rgba(0,0,0,0.95)_55%,#000_100%)]"
      />
      <div className="relative z-10 flex flex-col items-center px-6 text-center">
        <div className="mb-6 select-none text-4xl font-black tracking-tight text-(--color-accent) sm:text-5xl">
          {brandNameUpper()}
        </div>
        <div className="mb-7 max-w-sm text-base text-white/80">
          Preparing your library…
        </div>
        <div className="h-1 w-56 overflow-hidden rounded-full bg-white/10">
          <div className="zf-cs-bar h-full w-1/3 rounded-full bg-(--color-accent)" />
        </div>
        <div className="mt-6 max-w-xs text-xs text-white/45">
          This only happens once after a server restart while we cache your
          libraries.
        </div>
      </div>
      <style>{`
        @keyframes zf-cs-bar {
          0%   { transform: translateX(-100%); }
          100% { transform: translateX(280%); }
        }
        .zf-cs-bar {
          animation: zf-cs-bar 1.4s ease-in-out infinite;
        }
      `}</style>
    </div>
  );
}
