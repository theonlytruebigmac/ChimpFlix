"use client";

import { useEffect, useRef, useState } from "react";

interface Props {
  prerollUrl: string;
  /// Output level 0..=100. Applied to the video element before play()
  /// so the operator's chosen level takes effect on the *first* frame,
  /// not after a default-volume pop. Falls back to 100 if not provided.
  prerollVolume?: number;
  children: React.ReactNode;
}

/// Plays the operator-uploaded pre-roll once on mount, then unmounts
/// itself and reveals `children` (the real player). The skip button
/// appears as soon as the first frame plays — there's no point making
/// the viewer wait to be allowed to skip a sting they've seen 100x.
///
/// Layout: `fixed inset-0` so we cover the whole viewport regardless
/// of what the parent route's CSS does. Without this, the player
/// container can collapse to less than the viewport (the watch route
/// is plain page-flow, not `h-screen`), `object-contain` then centers
/// within that smaller box, and the result looks left-anchored / not
/// centered.
///
/// If the pre-roll fails to load (network blip, codec mismatch), we
/// skip straight through — never blocking playback on a sting.
export function PrerollGate({ prerollUrl, prerollVolume, children }: Props) {
  const [done, setDone] = useState(false);
  const [showSkip, setShowSkip] = useState(false);
  const videoRef = useRef<HTMLVideoElement | null>(null);

  // Autoplay best-effort — mobile + some desktop browsers reject
  // autoplay on unmuted media. The element is muted=false by intent
  // (pre-rolls usually have audio), but we set muted *after* the
  // first failed play attempt as a fallback, then unmute on user
  // interaction.
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    // Apply the operator-set volume before kicking off playback so the
    // first frame already plays at the correct level. Default 100 if
    // the prop is missing (older callers that don't know about volume
    // yet).
    const clamped = Math.max(0, Math.min(100, prerollVolume ?? 100));
    v.volume = clamped / 100;
    v.play().catch(() => {
      v.muted = true;
      v.play().catch(() => {
        // If even muted autoplay fails (rare), just skip — better
        // than leaving the viewer staring at a paused sting.
        setDone(true);
      });
    });
  }, [prerollVolume]);

  if (done) return <>{children}</>;

  return (
    <div className="fixed inset-0 z-50 bg-black">
      <video
        ref={videoRef}
        src={prerollUrl}
        playsInline
        // The skip button is enabled the moment the browser starts
        // pushing frames. `onPlaying` fires once playback has actually
        // begun (not on the autoplay-attempt promise) so users with a
        // slow-start codec never see a button they can't yet use.
        onPlaying={() => setShowSkip(true)}
        onEnded={() => setDone(true)}
        onError={() => setDone(true)}
        className="h-full w-full object-contain"
      />
      {showSkip && (
        <button
          type="button"
          onClick={() => setDone(true)}
          className="absolute bottom-8 right-8 rounded border border-white/50 bg-black/60 px-4 py-2 text-sm font-semibold text-white hover:bg-black/80"
        >
          Skip ›
        </button>
      )}
    </div>
  );
}
