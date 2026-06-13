"use client";

import { useEffect, useRef, useState } from "react";

interface Props {
  /// The sting to play, or `null`/empty when no pre-roll is configured
  /// (or we're resuming). When falsy the gate renders straight to
  /// `children` — the watch page always wraps the player in this
  /// component so the player's React parent type stays stable across
  /// `router.refresh()` and never remounts.
  prerollUrl: string | null;
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
  // With a pre-roll, `done` MUST start false to match the server render.
  // The binge-suppression decision reads localStorage + the clock, which
  // only exist on the client — doing it in the useState initializer made
  // the server render the bumper while the client's first render skipped
  // straight to the player, a top-level subtree divergence that threw
  // React #418 (hydration mismatch). Resolve suppression AFTER mount.
  //
  // With NO pre-roll (`prerollUrl` falsy) we start `done = true` so the
  // gate is a transparent pass-through to `children`. That's safe for
  // hydration because `prerollUrl` is a server-passed prop — identical on
  // the server and the first client render — so both sides render
  // `children` and there's no divergence.
  const [done, setDone] = useState(!prerollUrl);
  // Gates the autoplay effect so the bumper never starts before the
  // post-mount suppression check has had a chance to skip it — keeps the
  // skip flash-free without a render-time browser read.
  const [suppressChecked, setSuppressChecked] = useState(false);
  const [showSkip, setShowSkip] = useState(false);
  const videoRef = useRef<HTMLVideoElement | null>(null);

  // Binge-suppression: if this key's bumper was watched within the
  // window, skip straight to the player. Runs only on the client (an
  // effect), so SSR and the first client render agree (both render the
  // gate) and then this collapses it — no hydration mismatch.
  useEffect(() => {
    if (prerollKey) {
      const prev = readPrerollState();
      if (
        prev &&
        prev.key === prerollKey &&
        Date.now() - prev.at < PREROLL_SUPPRESS_MS
      ) {
        // eslint-disable-next-line react-hooks/set-state-in-effect
        setDone(true);
      }
    }
    setSuppressChecked(true);
  }, [prerollKey]);

  // Autoplay best-effort — mobile + some desktop browsers reject
  // autoplay on unmuted media. The element is muted=false by intent
  // (pre-rolls usually have audio), but we set muted *after* the
  // first failed play attempt as a fallback, then unmute on user
  // interaction.
  useEffect(() => {
    // Wait until the suppression check has run so we never start the
    // bumper on a play that should have been skipped. Also bail when
    // there's no pre-roll to play (the gate is a pass-through).
    if (done || !suppressChecked || !prerollUrl) return;
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
  }, [done, suppressChecked, prerollVolume, prerollUrl]);

  // Stamp the suppression cookie when the user actually watched the
  // pre-roll (ended or explicit skip). Stamping happens on transition,
  // not on every effect run, so the timestamp reflects the latest play.
  const stamp = () => {
    if (prerollKey) {
      writePrerollState({ key: prerollKey, at: Date.now() });
    }
    setDone(true);
  };
  // On load error (network blip, codec mismatch, server 5xx) skip the
  // gate without stamping — the user never saw the pre-roll, so the
  // suppression window should not start. Mirrors the autoplay-rejection
  // catch path at lines 141-149 which also skips stamping for the same
  // reason.
  const onVideoError = () => setDone(true);

  if (done) return <>{children}</>;

  return (
    <div className="fixed inset-0 z-50 bg-black">
      <video
        ref={videoRef}
        src={prerollUrl ?? undefined}
        playsInline
        // The skip button is enabled the moment the browser starts
        // pushing frames. `onPlaying` fires once playback has actually
        // begun (not on the autoplay-attempt promise) so users with a
        // slow-start codec never see a button they can't yet use.
        onPlaying={() => setShowSkip(true)}
        onEnded={stamp}
        onError={onVideoError}
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
