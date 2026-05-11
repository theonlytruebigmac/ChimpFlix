"use client";

import { useCallback, useEffect, useRef } from "react";
import { usePrefs } from "@/lib/prefs";

/**
 * Mounts a muted, looping YouTube embed for trailer autoplay. Uses the
 * youtube-nocookie domain so we don't drop tracking cookies. The iframe
 * itself has `pointer-events: none` so clicks pass through to surrounding
 * hero buttons; the audio toggle is a sibling element with normal pointer
 * events, and we drive mute/unmute via the YouTube IFrame API postMessage
 * channel (enablejsapi=1).
 *
 * Mute state is persisted via prefs.trailerMuted so unmuting once carries
 * across modals and sessions. The iframe always loads muted (browsers block
 * unmuted autoplay), and we apply the user's preference via postMessage as
 * soon as the iframe finishes loading.
 */
export function TrailerPlayer({
  videoId,
  className = "",
}: {
  videoId: string;
  className?: string;
}) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const [prefs, updatePrefs] = usePrefs();
  const muted = prefs.trailerMuted;

  const postCommand = useCallback((func: "mute" | "unMute") => {
    iframeRef.current?.contentWindow?.postMessage(
      JSON.stringify({ event: "command", func, args: [] }),
      "*",
    );
  }, []);

  function toggleMute() {
    const next = !muted;
    updatePrefs({ trailerMuted: next });
    postCommand(next ? "mute" : "unMute");
  }

  // Re-apply when pref changes from another tab or component instance.
  useEffect(() => {
    postCommand(muted ? "mute" : "unMute");
  }, [muted, postCommand]);

  function onLoad() {
    // Apply current pref once the embed is alive — required because the
    // initial src must include mute=1 for autoplay to work.
    if (!muted) postCommand("unMute");
  }

  const params = new URLSearchParams({
    autoplay: "1",
    mute: "1",
    enablejsapi: "1",
    controls: "0",
    loop: "1",
    playlist: videoId, // required for `loop=1` to actually loop
    modestbranding: "1",
    rel: "0",
    iv_load_policy: "3",
    playsinline: "1",
    disablekb: "1",
  });
  const src = `https://www.youtube-nocookie.com/embed/${videoId}?${params}`;

  return (
    <div className={`relative ${className}`}>
      <iframe
        ref={iframeRef}
        src={src}
        title="Trailer"
        allow="autoplay; encrypted-media"
        onLoad={onLoad}
        className="pointer-events-none absolute inset-0 h-full w-full"
        // Slight scale-up crops YouTube's "Watch on YouTube" overlay corners
        // without losing meaningful video content.
        style={{
          border: 0,
          transform: "scale(1.4)",
          transformOrigin: "center",
        }}
      />
      <button
        type="button"
        onClick={toggleMute}
        aria-label={muted ? "Unmute trailer" : "Mute trailer"}
        className="absolute bottom-3 right-3 z-30 flex h-10 w-10 items-center justify-center rounded-full border-2 border-white/60 bg-black/40 text-white backdrop-blur-sm transition-colors hover:border-white hover:bg-black/60"
      >
        {muted ? <MutedIcon /> : <SoundIcon />}
      </button>
    </div>
  );
}

function MutedIcon() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" fill="currentColor" />
      <line x1="22" y1="9" x2="16" y2="15" />
      <line x1="16" y1="9" x2="22" y2="15" />
    </svg>
  );
}

function SoundIcon() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" fill="currentColor" />
      <path d="M15.5 8.5a4 4 0 0 1 0 7" />
      <path d="M18.5 5.5a8 8 0 0 1 0 13" />
    </svg>
  );
}
