"use client";

import { useEffect, useRef, useState } from "react";

interface Props {
  prerollUrl: string;
  children: React.ReactNode;
}

/// Plays the operator-uploaded pre-roll once on mount, then unmounts
/// itself and reveals `children` (the real player). A 5-second
/// skip button appears once the pre-roll is past the half-way point
/// of those five seconds, so impatient viewers aren't held captive.
///
/// If the pre-roll fails to load (network blip, codec mismatch), we
/// skip straight through — never blocking playback on a sting.
export function PrerollGate({ prerollUrl, children }: Props) {
  const [done, setDone] = useState(false);
  const [showSkip, setShowSkip] = useState(false);
  const videoRef = useRef<HTMLVideoElement | null>(null);

  useEffect(() => {
    const t = window.setTimeout(() => setShowSkip(true), 5000);
    return () => window.clearTimeout(t);
  }, []);

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
    <div className="relative h-full w-full bg-black">
      <video
        ref={videoRef}
        src={prerollUrl}
        playsInline
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
