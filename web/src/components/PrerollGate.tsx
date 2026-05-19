"use client";

import { useEffect, useRef, useState } from "react";

interface Props {
  prerollUrl: string;
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
export function PrerollGate({ prerollUrl, children }: Props) {
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
    v.play().catch(() => {
      v.muted = true;
      v.play().catch(() => {
        // If even muted autoplay fails (rare), just skip — better
        // than leaving the viewer staring at a paused sting.
        setDone(true);
      });
    });
  }, []);

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
