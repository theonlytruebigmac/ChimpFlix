"use client";

import { useEffect, useState } from "react";
import { TrailerPlayer } from "./TrailerPlayer";

/**
 * Looks up a YouTube trailer for the given title and renders it as an absolute
 * overlay over the hero, fading in after `delayMs`. Sits below the gradient
 * and content via z-index so the title/buttons stay readable.
 */
export function HeroTrailer({
  type,
  title,
  year,
  delayMs = 3000,
}: {
  type: "movie" | "show";
  title: string;
  year?: number;
  delayMs?: number;
}) {
  const [videoId, setVideoId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;

    const params = new URLSearchParams({
      type: type === "show" ? "tv" : "movie",
      title,
    });
    if (year) params.set("year", String(year));

    fetch(`/api/tmdb/trailer?${params}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((data: { videoId?: string | null } | null) => {
        if (cancelled || !data?.videoId) return;
        timer = window.setTimeout(() => {
          if (!cancelled) setVideoId(data.videoId ?? null);
        }, delayMs);
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [type, title, year, delayMs]);

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
