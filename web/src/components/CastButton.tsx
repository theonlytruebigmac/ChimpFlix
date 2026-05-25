"use client";

import { useCallback, type RefObject } from "react";
import {
  endCastSession,
  showAirPlayPicker,
  startCastSession,
  useAirPlayAvailability,
  useCastState,
  type CastMediaPayload,
} from "@/lib/cast";

/// Unified Cast / AirPlay affordance.
///
/// Renders a single icon button that opens whichever picker the
/// platform supports. Hidden entirely when neither Cast (Chromium)
/// nor AirPlay (Safari/iOS) is available — the player toolbar shouldn't
/// carry a button that does nothing.
///
/// When a Cast session is already live, the button doubles as a "stop
/// casting" toggle.
export interface CastButtonProps {
  videoRef: RefObject<HTMLVideoElement | null>;
  /// Called when the user has picked a Cast device and the receiver
  /// is loading our media. The player should pause local playback +
  /// show a "casting to <device>" overlay.
  onCastStart?: () => void;
  /// Called when the cast session ends (user clicked stop, receiver
  /// went away, etc.). The player should resume local playback.
  onCastEnd?: () => void;
  /// Lazily build the media payload at click time so the URL / token
  /// reflect the latest playback position rather than a stale snapshot
  /// from when the button mounted.
  resolveMedia: () => Promise<CastMediaPayload | null>;
}

export function CastButton({
  videoRef,
  onCastStart,
  onCastEnd,
  resolveMedia,
}: CastButtonProps) {
  const cast = useCastState();
  const airplayAvailable = useAirPlayAvailability(videoRef);

  const onClick = useCallback(async () => {
    if (cast.connected) {
      endCastSession(true);
      onCastEnd?.();
      return;
    }
    if (cast.available) {
      const media = await resolveMedia();
      if (!media) return;
      const ok = await startCastSession(media);
      if (ok) onCastStart?.();
      return;
    }
    // Cast SDK not available — fall through to AirPlay.
    if (airplayAvailable) {
      const v = videoRef.current;
      if (v) showAirPlayPicker(v);
    }
  }, [
    cast.available,
    cast.connected,
    airplayAvailable,
    videoRef,
    resolveMedia,
    onCastStart,
    onCastEnd,
  ]);

  // Hide entirely if neither protocol is usable. A button that
  // sometimes does nothing teaches the user not to click it.
  if (!cast.available && !airplayAvailable) return null;

  const label = cast.connected
    ? `Stop casting${cast.deviceName ? ` to ${cast.deviceName}` : ""}`
    : cast.available
      ? cast.hasDevices
        ? "Cast to device"
        : "No cast devices found"
      : "AirPlay";

  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={() => void onClick()}
      className="flex h-10 w-10 items-center justify-center rounded text-white/80 outline-none transition hover:bg-white/10 hover:text-white focus-visible:ring-2 focus-visible:ring-(--color-accent)"
    >
      {cast.available ? (
        // Cast glyph — three concentric arcs + a screen rectangle.
        // Matches the standard Chromecast button silhouette.
        <svg
          width="22"
          height="22"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <path d="M2 8V6a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-6" />
          <path d="M2 12a8 8 0 0 1 8 8" />
          <path d="M2 16a4 4 0 0 1 4 4" />
          <circle cx="3" cy="20" r="1.2" fill="currentColor" stroke="none" />
          {cast.connected && (
            <rect x="4" y="6" width="14" height="10" fill="currentColor" stroke="none" rx="1" />
          )}
        </svg>
      ) : (
        // AirPlay glyph — triangle pointing up out of a rectangle.
        <svg
          width="22"
          height="22"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <path d="M5 17H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-1" />
          <polygon points="12 14 18 21 6 21" fill="currentColor" stroke="currentColor" />
        </svg>
      )}
    </button>
  );
}
