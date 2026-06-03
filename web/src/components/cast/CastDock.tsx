"use client";

/// App-wide cast surface. Mounted once in the root layout (like
/// TopNavGate / LiveRefresh), it:
///   * shows the persistent mini-controller on browse/detail screens
///     while a session is live, and a full-screen expanded controller
///     when the user taps to open it; and
///   * owns `navigator.mediaSession` while casting — metadata + position
///     + transport action handlers — so Android's "now playing" chip
///     shows the right title/art and its buttons drive the TV.
///
/// On the immersive `/watch` route the player renders its OWN embedded
/// remote, so the dock stays out of the way there (no duplicate mini bar)
/// but STILL owns mediaSession (single owner; the player's local
/// mediaSession effect yields while casting).

import { useEffect, useState } from "react";
import { usePathname } from "next/navigation";
import {
  castPlayPause,
  castSeekToMediaTime,
  endCastSession,
  getRemoteSnapshot,
  useRemotePlayback,
} from "@/lib/cast";
import { CastRemote } from "./CastRemote";
import { MiniController } from "./MiniController";

export function CastDock() {
  const remote = useRemotePlayback();
  const pathname = usePathname() ?? "";
  const [expanded, setExpanded] = useState(false);

  const connected = remote.isConnected;
  // The player owns the on-screen remote on /watch; the dock only shows
  // its mini/expanded surface elsewhere. When disconnected or on /watch
  // we render nothing, so a stale `expanded=true` is simply never shown
  // (derived here rather than reset in an effect, which would cascade).
  const onWatch = pathname === "/watch" || pathname.startsWith("/watch/");
  const showExpanded = connected && !onWatch && expanded;

  // --- Media Session: action handlers, registered once per connection. ---
  useEffect(() => {
    if (!connected) return;
    if (typeof navigator === "undefined" || !("mediaSession" in navigator)) {
      return;
    }
    const ms = navigator.mediaSession;
    const onSeekBackward = (d: MediaSessionActionDetails) => {
      const offset = d.seekOffset ?? 10;
      castSeekToMediaTime(Math.max(0, getRemoteSnapshot().currentTimeS - offset));
    };
    const onSeekForward = (d: MediaSessionActionDetails) => {
      const offset = d.seekOffset ?? 10;
      castSeekToMediaTime(getRemoteSnapshot().currentTimeS + offset);
    };
    const onSeekTo = (d: MediaSessionActionDetails) => {
      if (typeof d.seekTime === "number") castSeekToMediaTime(d.seekTime);
    };
    const set = (action: MediaSessionAction, handler: MediaSessionActionHandler | null) => {
      try {
        ms.setActionHandler(action, handler);
      } catch {
        // Browser doesn't support this action type — ignore.
      }
    };
    set("play", castPlayPause);
    set("pause", castPlayPause);
    set("seekbackward", onSeekBackward);
    set("seekforward", onSeekForward);
    set("seekto", onSeekTo);
    set("stop", () => endCastSession(true));
    return () => {
      set("play", null);
      set("pause", null);
      set("seekbackward", null);
      set("seekforward", null);
      set("seekto", null);
      set("stop", null);
    };
  }, [connected]);

  // --- Media Session: metadata (title + artwork), updated when it changes. ---
  useEffect(() => {
    if (!connected) return;
    if (typeof navigator === "undefined" || !("mediaSession" in navigator)) {
      return;
    }
    if (typeof MediaMetadata === "undefined") return;
    navigator.mediaSession.metadata = new MediaMetadata({
      title: remote.title ?? "ChimpFlix",
      artist: remote.deviceName ? `Casting · ${remote.deviceName}` : "Casting",
      artwork: remote.imageUrl
        ? [{ src: remote.imageUrl, sizes: "512x512", type: "image/jpeg" }]
        : undefined,
    });
  }, [connected, remote.title, remote.deviceName, remote.imageUrl]);

  // --- Media Session: playback + position, so the chip scrubber tracks the TV. ---
  useEffect(() => {
    if (!connected) return;
    if (typeof navigator === "undefined" || !("mediaSession" in navigator)) {
      return;
    }
    navigator.mediaSession.playbackState = remote.isPaused
      ? "paused"
      : "playing";
    if (
      typeof navigator.mediaSession.setPositionState === "function" &&
      remote.durationS > 0 &&
      remote.currentTimeS <= remote.durationS
    ) {
      try {
        navigator.mediaSession.setPositionState({
          duration: remote.durationS,
          position: Math.max(0, remote.currentTimeS),
          playbackRate: 1,
        });
      } catch {
        // Invalid state (e.g. duration shrank between ticks) — skip.
      }
    }
  }, [connected, remote.isPaused, remote.currentTimeS, remote.durationS]);

  if (!connected || onWatch) return null;

  return (
    <>
      {showExpanded ? (
        <div className="fixed inset-0 z-50">
          <CastRemote variant="page" onCollapse={() => setExpanded(false)} />
        </div>
      ) : (
        <MiniController onExpand={() => setExpanded(true)} />
      )}
    </>
  );
}
