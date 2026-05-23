"use client";

import { useEffect, useRef, useState } from "react";

interface Props {
  prerollUrl: string;
  /// Output level 0..=100. Applied to the video element before play()
  /// so the operator's chosen level takes effect on the *first* frame,
  /// not after a default-volume pop. Falls back to 100 if not provided.
  prerollVolume?: number;
  /// Identity used to suppress the pre-roll for back-to-back episodes
  /// of the same show, or repeated plays of the same movie inside a
  /// "binge" window. Set to `show:<id>` for a TV episode (so every
  /// episode of the show shares the same key) and `item:<id>` for a
  /// movie. When omitted, the pre-roll always plays.
  prerollKey?: string;
  children: React.ReactNode;
}

/// localStorage key holding `{key, at}` for the most recent pre-roll
/// the user actually watched. Mirrors Netflix's "show the bumper once
/// per binge" behaviour — once you've sat through it, follow-up
/// episodes within the window get straight to playback.
const PREROLL_STATE_KEY = "chimpflix:preroll-state";
/// Suppression window. Comfortably covers a multi-episode binge or
/// a movie + sequel marathon while still re-triggering when the user
/// comes back the next day. Tuned at 6h.
const PREROLL_SUPPRESS_MS = 6 * 60 * 60 * 1000;

type PrerollState = { key: string; at: number };

function readPrerollState(): PrerollState | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(PREROLL_STATE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<PrerollState>;
    if (
      typeof parsed.key !== "string" ||
      typeof parsed.at !== "number" ||
      !Number.isFinite(parsed.at)
    ) {
      return null;
    }
    return { key: parsed.key, at: parsed.at };
  } catch {
    return null;
  }
}

function writePrerollState(state: PrerollState) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(PREROLL_STATE_KEY, JSON.stringify(state));
  } catch {
    /* localStorage may be disabled — best-effort */
  }
}

/// Plays the operator-uploaded pre-roll once on mount, then unmounts
/// itself and reveals `children` (the real player). The skip button
/// appears as soon as the first frame plays — there's no point making
/// the viewer wait to be allowed to skip a sting they've seen 100x.
///
/// Suppression: when `prerollKey` matches the last-watched key within
/// PREROLL_SUPPRESS_MS, the gate resolves to children immediately.
/// Lets a TV binge or movie marathon stop replaying the sting on
/// every Up Next transition. Re-triggers on a different show / movie
/// or after the window expires.
///
/// If the pre-roll fails to load (network blip, codec mismatch), we
/// skip straight through — never blocking playback on a sting.
export function PrerollGate({
  prerollUrl,
  prerollVolume,
  prerollKey,
  children,
}: Props) {
  // Decide synchronously on first render whether to skip. Doing this
  // in an effect would briefly flash the bumper while React commits.
  const [done, setDone] = useState<boolean>(() => {
    if (!prerollKey || typeof window === "undefined") return false;
    const prev = readPrerollState();
    if (!prev) return false;
    if (prev.key !== prerollKey) return false;
    return Date.now() - prev.at < PREROLL_SUPPRESS_MS;
  });
  const [showSkip, setShowSkip] = useState(false);
  const videoRef = useRef<HTMLVideoElement | null>(null);

  // Autoplay best-effort — mobile + some desktop browsers reject
  // autoplay on unmuted media. The element is muted=false by intent
  // (pre-rolls usually have audio), but we set muted *after* the
  // first failed play attempt as a fallback, then unmute on user
  // interaction.
  useEffect(() => {
    if (done) return;
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
        // than leaving the viewer staring at a paused sting. Don't
        // stamp the suppression cookie here: the user didn't actually
        // see the pre-roll, so we shouldn't pretend they did.
        setDone(true);
      });
    });
  }, [done, prerollVolume]);

  // Stamp the suppression cookie whenever the gate completes — covers
  // both ended/error paths and explicit skip clicks. Stamping happens
  // on transition, not on every effect run, so the timestamp reflects
  // the latest play rather than the first.
  const stamp = () => {
    if (prerollKey) {
      writePrerollState({ key: prerollKey, at: Date.now() });
    }
    setDone(true);
  };

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
        onEnded={stamp}
        onError={stamp}
        className="h-full w-full object-contain"
      />
      {showSkip && (
        <button
          type="button"
          onClick={stamp}
          className="absolute bottom-8 right-8 rounded border border-white/50 bg-black/60 px-4 py-2 text-sm font-semibold text-white hover:bg-black/80"
        >
          Skip ›
        </button>
      )}
    </div>
  );
}
