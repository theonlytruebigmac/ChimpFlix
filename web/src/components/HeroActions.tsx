"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useRef } from "react";
import { openModal } from "@/lib/modal";
import { prefetchPlay } from "@/lib/play-prefetch";
import { cancelPrewarm, prewarmFor } from "@/lib/prewarm";
import { detectClientCapabilities } from "@/lib/client-caps";
import { getPrefs } from "@/lib/prefs";

export function HeroActions({
  playRatingKey,
  modalRatingKey,
  modalEpisodeKey,
}: {
  // What to play. For an episode-type hero this is the episode itself; for
  // a show/movie it's the title.
  playRatingKey: string;
  // What to open in the modal. For an episode-type hero this is the
  // grandparent show, so the user sees the season/episode list. Movies
  // and shows just use their own key.
  modalRatingKey: string;
  // For an episode-type hero, the episode rating key (`e<id>`) so the
  // modal can land on the right season + scroll the row in.
  modalEpisodeKey?: string;
}) {
  const router = useRouter();

  // The Hero Play button is always above the fold, so its target route is
  // an obvious candidate to prefetch as soon as we render. router.prefetch
  // is a no-op if Next has already auto-prefetched.
  useEffect(() => {
    router.prefetch(`/watch/${playRatingKey}`);
  }, [router, playRatingKey]);

  // Hover-time session pre-warm — see TitleModalClient for the same
  // pattern. The hero's Play button is the highest-conviction click
  // surface in the app (one tap from landing on the home page) so the
  // user-experience win for the cached case is biggest here.
  const prewarmTimerRef = useRef<number | null>(null);
  const startPrewarm = () => {
    if (prewarmTimerRef.current !== null) return;
    prewarmTimerRef.current = window.setTimeout(() => {
      prewarmTimerRef.current = null;
      try {
        const caps = detectClientCapabilities();
        prewarmFor(
          playRatingKey,
          {
            supported_video_codecs: caps.video,
            supported_audio_codecs: caps.audio,
            supported_containers: caps.containers,
          },
          getPrefs().audioNormalize,
        );
      } catch {
        // Best-effort.
      }
    }, 250);
  };
  const cancelPendingPrewarm = () => {
    if (prewarmTimerRef.current !== null) {
      window.clearTimeout(prewarmTimerRef.current);
      prewarmTimerRef.current = null;
    }
  };
  useEffect(() => {
    return () => {
      cancelPendingPrewarm();
      void cancelPrewarm();
    };
  }, []);

  return (
    <div className="flex gap-3">
      <Link
        href={`/watch/${playRatingKey}`}
        onMouseEnter={() => {
          prefetchPlay();
          startPrewarm();
        }}
        onMouseLeave={cancelPendingPrewarm}
        onFocus={() => {
          prefetchPlay();
          startPrewarm();
        }}
        onBlur={cancelPendingPrewarm}
        className="inline-flex items-center gap-2 rounded-md bg-white px-7 py-2.5 text-base font-bold text-black transition-colors hover:bg-white/85"
      >
        <PlayIcon /> Play
      </Link>
      <button
        type="button"
        onClick={() => openModal(modalRatingKey, modalEpisodeKey)}
        className="inline-flex cursor-pointer items-center gap-2 rounded-md bg-white/25 px-7 py-2.5 text-base font-bold text-white backdrop-blur-sm transition-colors hover:bg-white/35"
      >
        <InfoIcon /> More Info
      </button>
    </div>
  );
}

function PlayIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M6 4l14 8-14 8V4z" />
    </svg>
  );
}

function InfoIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      aria-hidden
    >
      <circle cx="12" cy="12" r="9" />
      <line x1="12" y1="11" x2="12" y2="17" />
      <circle cx="12" cy="7.5" r="1" fill="currentColor" stroke="none" />
    </svg>
  );
}
