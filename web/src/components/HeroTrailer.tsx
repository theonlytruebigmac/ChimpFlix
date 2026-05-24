"use client";

import { useEffect, useState } from "react";
import { TrailerPlayer } from "./TrailerPlayer";
import { items as itemsApi } from "@/lib/chimpflix-api";

/**
 * Looks up a YouTube trailer for the given item via Rust and renders it
 * as an absolute overlay over the hero, fading in after `delayMs`. Sits
 * below the gradient and content via z-index so the title/buttons stay
 * readable. Silent no-op when the item has no trailer or no tmdb_id.
 */
export function HeroTrailer({
  ratingKey,
  delayMs = 3000,
}: {
  ratingKey: string;
  delayMs?: number;
}) {
  const [videoId, setVideoId] = useState<string | null>(null);

  useEffect(() => {
    const id = Number.parseInt(ratingKey, 10);
    if (!Number.isFinite(id) || id <= 0) return;
    let cancelled = false;
    let timer: number | undefined;

    itemsApi
      .trailer(id)
      .then((data) => {
        if (cancelled || !data.video_id) return;
        timer = window.setTimeout(() => {
          if (!cancelled) setVideoId(data.video_id ?? null);
        }, delayMs);
      })
      .catch(() => {
        // Trailer lookup is best-effort polish — TMDB outages or
        // 4xxs shouldn't break the hero. Fall through to the
        // backdrop-only hero (videoId stays null → component
        // renders nothing).
      });

    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [ratingKey, delayMs]);

  if (!videoId) return null;

  return (
    <div className="absolute inset-0 animate-[fadein_1s_ease-in_forwards] opacity-0">
      <TrailerPlayer
        videoId={videoId}
        className="absolute inset-0 h-full w-full"
      />
      <style>{`
        @keyframes fadein {
          from { opacity: 0; }
          to { opacity: 1; }
        }
      `}</style>
    </div>
  );
}
