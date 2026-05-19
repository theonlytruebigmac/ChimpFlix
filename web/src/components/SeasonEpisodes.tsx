"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { formatRuntime, type MediaItem } from "@/lib/chimpflix-types";
import { plexImage } from "@/lib/image";

export function SeasonEpisodes({
  seasons,
  initialEpisodes,
  initialSeasonKey,
}: {
  seasons: MediaItem[];
  initialEpisodes: MediaItem[];
  initialSeasonKey: string;
}) {
  const [selectedKey, setSelectedKey] = useState(initialSeasonKey);
  const [episodes, setEpisodes] = useState(initialEpisodes);
  const [loading, setLoading] = useState(false);
  // Monotonic request id + alive flag. Used to drop stale responses
  // — a rapid user double-click on different seasons used to race
  // the two fetches and whichever resolved last won, even if it was
  // the earlier click. Now only the most-recent request's response
  // is applied. The alive flag also prevents setState after unmount.
  const requestIdRef = useRef(0);
  const aliveRef = useRef(true);
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  async function changeSeason(seasonKey: string) {
    if (seasonKey === selectedKey) return;
    const myRequestId = ++requestIdRef.current;
    setSelectedKey(seasonKey);
    setLoading(true);
    try {
      // seasonKey is "s<id>" per the Rust adapter; strip the prefix to
      // hit /api/v1/seasons/<id>.
      const numericId = seasonKey.startsWith("s") ? seasonKey.slice(1) : seasonKey;
      const res = await fetch(`/api/v1/seasons/${numericId}`);
      if (!aliveRef.current || requestIdRef.current !== myRequestId) return;
      if (!res.ok) {
        setEpisodes([]);
        return;
      }
      const data = (await res.json()) as {
        episodes: {
          id: number;
          season_number: number;
          episode_number: number;
          title: string;
          summary: string | null;
          duration_ms: number | null;
          thumb_path: string | null;
          play_state: { position_ms: number } | null;
        }[];
      };
      if (!aliveRef.current || requestIdRef.current !== myRequestId) return;
      setEpisodes(
        data.episodes.map((e) => ({
          ratingKey: `e${e.id}`,
          key: `/episodes/${e.id}`,
          type: "episode",
          title: e.title,
          summary: e.summary ?? undefined,
          thumb: e.thumb_path ?? undefined,
          duration: e.duration_ms ?? undefined,
          viewOffset: e.play_state?.position_ms ?? undefined,
          index: e.episode_number,
        })),
      );
    } finally {
      if (aliveRef.current && requestIdRef.current === myRequestId) {
        setLoading(false);
      }
    }
  }

  const selectedSeason = seasons.find((s) => s.ratingKey === selectedKey);

  return (
    <section className="border-t border-white/10 px-4 sm:px-8 md:px-12 py-8">
      <div className="mb-6 flex items-center justify-between">
        <h2 className="text-2xl font-medium">Episodes</h2>
        {seasons.length > 1 && (
          <div className="relative">
            <select
              value={selectedKey}
              onChange={(e) => changeSeason(e.target.value)}
              className="appearance-none rounded-md border border-white/30 bg-(--color-surface) py-2 pl-4 pr-10 text-base font-medium hover:border-white/60 focus:outline-none"
            >
              {seasons.map((s) => (
                <option key={s.ratingKey} value={s.ratingKey}>
                  {s.title}
                </option>
              ))}
            </select>
            <svg
              className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              aria-hidden
            >
              <polyline points="6 9 12 15 18 9" />
            </svg>
          </div>
        )}
      </div>

      {selectedSeason?.summary && (
        <p className="mb-6 max-w-3xl text-sm text-white/70">
          {selectedSeason.summary}
        </p>
      )}

      <ul
        className={`divide-y divide-white/10 transition-opacity ${
          loading ? "opacity-50" : "opacity-100"
        }`}
      >
        {episodes.map((ep, idx) => {
          const thumb = plexImage(ep.thumb ?? ep.art, 320, 180);
          const progress =
            ep.viewOffset && ep.duration
              ? Math.min(100, (ep.viewOffset / ep.duration) * 100)
              : null;

          return (
            <li key={ep.ratingKey}>
              <Link
                href={`/watch/${ep.ratingKey}`}
                className="group -mx-3 flex gap-4 rounded-md px-3 py-5 transition-colors hover:bg-white/5"
              >
                <div className="flex w-8 shrink-0 items-start pt-2 text-2xl font-medium text-white/60">
                  {idx + 1}
                </div>
                <div className="relative aspect-video w-44 shrink-0 overflow-hidden rounded bg-black">
                  {thumb && (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img
                      src={thumb}
                      alt=""
                      className="h-full w-full object-cover transition-opacity group-hover:opacity-70"
                      loading="lazy"
                    />
                  )}
                  <div className="absolute inset-0 flex items-center justify-center">
                    <div className="flex h-11 w-11 items-center justify-center rounded-full border-2 border-white/80 bg-black/40 text-white opacity-90 backdrop-blur-sm transition-all group-hover:scale-110 group-hover:border-white group-hover:bg-white group-hover:text-black group-hover:opacity-100">
                      <svg
                        width="18"
                        height="18"
                        viewBox="0 0 24 24"
                        fill="currentColor"
                        aria-hidden
                      >
                        <path d="M7 4l13 8-13 8V4z" />
                      </svg>
                    </div>
                  </div>
                  {progress !== null && (
                    <div className="absolute inset-x-2 bottom-1.5 h-0.75 rounded-full bg-white/25">
                      <div
                        className="h-full rounded-full bg-(--color-accent)"
                        style={{ width: `${progress}%` }}
                      />
                    </div>
                  )}
                </div>
                <div className="flex-1">
                  <div className="mb-1 flex items-baseline justify-between gap-3">
                    <h3 className="text-base font-medium">{ep.title}</h3>
                    {ep.duration && (
                      <span className="shrink-0 text-sm text-white/60">
                        {formatRuntime(ep.duration)}
                      </span>
                    )}
                  </div>
                  {ep.summary && (
                    <p className="line-clamp-3 text-sm text-white/70">
                      {ep.summary}
                    </p>
                  )}
                </div>
              </Link>
            </li>
          );
        })}
        {episodes.length === 0 && !loading && (
          <li className="py-6 text-sm text-white/60">
            No episodes in this season.
          </li>
        )}
      </ul>
    </section>
  );
}
