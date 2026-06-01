"use client";

import { useEffect, useRef } from "react";
import { usePathname, useRouter } from "next/navigation";

/**
 * Live freshness without polling.
 *
 * Subscribes to the server's WebSocket event stream and re-renders the
 * current route's Server Components when content the user can see
 * changes:
 *   - `library_changed` — a scan completed (broadcast to everyone; the
 *     re-fetch is access-filtered server-side).
 *   - `playstate_changed` — the user's own watch progress changed on
 *     another tab/device (scoped to them by the server).
 *
 * This replaces reliance on manual reload: the home and library rails
 * stay fresh on their own. Connection is best-effort — it reconnects
 * with backoff and never throws into render (returns `null`).
 */
export function LiveRefresh() {
  const router = useRouter();
  // Read the live pathname through a ref so the long-lived socket effect
  // (deps: [router]) can consult the *current* route without tearing down
  // and re-opening the WebSocket on every client navigation.
  const pathname = usePathname();
  const pathnameRef = useRef(pathname);
  useEffect(() => {
    pathnameRef.current = pathname;
  }, [pathname]);
  // Coalesce bursts (a scan can emit several events) into one refresh.
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let socket: WebSocket | null = null;
    let closed = false;
    let backoff = 1000;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const scheduleRefresh = () => {
      // Never refresh the Server Components under an actively-playing
      // player. `router.refresh()` re-runs the /watch route's server
      // render, which re-derives the player's props from the *live,
      // advancing* play_state — flipping the PrerollGate wrapper and
      // remounting <ChimpFlixPlayer>. That remount drops fullscreen,
      // re-shows the "Resumed from…" pill, and autoplays a paused video.
      // The watch page is self-contained (the player owns its own live
      // state), so it has nothing to gain from a push-refresh anyway.
      if (pathnameRef.current?.startsWith("/watch/")) return;
      if (pending.current) return;
      pending.current = setTimeout(() => {
        pending.current = null;
        // Re-check on fire: the user may have navigated into /watch
        // during the 800ms coalescing window.
        if (pathnameRef.current?.startsWith("/watch/")) return;
        router.refresh();
      }, 800);
    };

    const scheduleReconnect = () => {
      if (closed || reconnectTimer) return;
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
        connect();
      }, backoff);
      backoff = Math.min(backoff * 2, 30000);
    };

    const connect = () => {
      if (closed) return;
      const proto = window.location.protocol === "https:" ? "wss" : "ws";
      const url = `${proto}://${window.location.host}/api/v1/ws`;
      try {
        socket = new WebSocket(url);
      } catch {
        scheduleReconnect();
        return;
      }
      socket.onopen = () => {
        backoff = 1000;
      };
      socket.onmessage = (e) => {
        try {
          const data = JSON.parse(e.data as string);
          if (data && data.type === "refresh") {
            scheduleRefresh();
          }
        } catch {
          // Non-JSON or unrelated frame — ignore.
        }
      };
      socket.onclose = () => {
        socket = null;
        scheduleReconnect();
      };
      socket.onerror = () => {
        socket?.close();
      };
    };

    connect();

    return () => {
      closed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      if (pending.current) clearTimeout(pending.current);
      socket?.close();
    };
  }, [router]);

  return null;
}
