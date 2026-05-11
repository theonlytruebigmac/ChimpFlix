"use client";

import { useEffect, useState } from "react";
import { brandNameUpper } from "@/lib/env";
import {
  displayTitle,
  formatRuntime,
  type MediaItem,
} from "@/lib/plex-types";
import { plexImage } from "@/lib/image";
import { openModal } from "@/lib/modal";
import {
  getOrFetchModalData,
  prefetchModalData,
  type ModalData,
} from "@/lib/modal-cache";
import { useMyListItem } from "@/lib/my-list";
import { prefetchPlay } from "@/lib/play-prefetch";
import { TitleModalShell } from "./TitleModalShell";
import { SeasonEpisodes } from "./SeasonEpisodes";
import { TrailerPlayer } from "./TrailerPlayer";

function useTrailer(item: MediaItem): string | null {
  const [videoId, setVideoId] = useState<string | null>(null);

  useEffect(() => {
    if (item.type !== "movie" && item.type !== "show") return;
    const tmdbType = item.type === "show" ? "tv" : "movie";
    let cancelled = false;
    let timer: number | undefined;

    const params = new URLSearchParams({
      type: tmdbType,
      title: item.title,
    });
    if (item.year) params.set("year", String(item.year));

    fetch(`/api/tmdb/trailer?${params}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((data: { videoId?: string | null } | null) => {
        if (cancelled || !data?.videoId) return;
        // Brief delay before swapping the static art for the trailer so the
        // user has time to read the title and synopsis.
        timer = window.setTimeout(() => {
          if (!cancelled) setVideoId(data.videoId ?? null);
        }, 2000);
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [item.title, item.year, item.type]);

  return videoId;
}

export function TitleModalClient({ ratingKey }: { ratingKey: string }) {
  const [data, setData] = useState<ModalData | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setData(null);
    setError(null);

    getOrFetchModalData(ratingKey)
      .then((d) => {
        if (cancelled) return;
        if (!d) {
          setError("Title not found");
          return;
        }
        setData(d);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      });

    return () => {
      cancelled = true;
    };
  }, [ratingKey]);

  return (
    <TitleModalShell>
      {data ? (
        <TitleModalView data={data} />
      ) : error ? (
        <div className="flex aspect-video items-center justify-center text-white/60">
          {error}
        </div>
      ) : (
        <ModalSkeleton />
      )}
    </TitleModalShell>
  );
}

function ModalSkeleton() {
  return (
    <div className="aspect-video w-full animate-pulse bg-linear-to-b from-white/5 to-(--color-surface)" />
  );
}

function TitleModalView({ data }: { data: ModalData }) {
  const { item, seasons, initialEpisodes, similar } = data;
  const isShow = item.type === "show";
  const { inList, toggle: toggleMyList } = useMyListItem(item.ratingKey);
  const trailerVideoId = useTrailer(item);
  const backdrop = plexImage(item.art ?? item.thumb, 1920, 1080);
  const title = displayTitle(item);
  const progress =
    item.viewOffset && item.duration
      ? Math.min(100, (item.viewOffset / item.duration) * 100)
      : null;
  const remainingMs =
    item.viewOffset && item.duration
      ? item.duration - item.viewOffset
      : undefined;

  const topCast = item.cast?.slice(0, 3) ?? [];
  const extraCast = (item.cast?.length ?? 0) > 3;
  const firstSeason = seasons[0];

  return (
    <>
      <div className="relative aspect-video w-full overflow-hidden">
        {backdrop && (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={backdrop}
            alt=""
            className={`zf-fade-in absolute inset-0 h-full w-full object-cover transition-opacity duration-500 ${
              trailerVideoId ? "opacity-0" : "opacity-100"
            }`}
          />
        )}
        {trailerVideoId && (
          <TrailerPlayer
            videoId={trailerVideoId}
            className="absolute inset-0 h-full w-full"
          />
        )}
        <div className="absolute inset-0 bg-linear-to-t from-surface via-surface/30 to-transparent" />

        <div className="absolute inset-x-0 bottom-0 p-10">
          <div className="mb-2 text-xs font-bold tracking-[0.35em] text-(--color-accent)">
            {brandNameUpper()}
          </div>
          <h1 className="mb-5 max-w-3xl text-5xl font-black uppercase leading-[0.95] tracking-tight drop-shadow-lg">
            {title}
          </h1>

          {progress !== null && remainingMs !== undefined && (
            <div className="mb-5 flex max-w-md items-center gap-3">
              <div className="h-1 flex-1 rounded-full bg-white/25">
                <div
                  className="h-full rounded-full bg-(--color-accent)"
                  style={{ width: `${progress}%` }}
                />
              </div>
              <div className="shrink-0 text-sm text-white/85">
                {formatRuntime(remainingMs)} left
              </div>
            </div>
          )}

          <div className="flex items-center gap-3">
            <a
              href={`/watch/${item.ratingKey}`}
              onMouseEnter={prefetchPlay}
              onFocus={prefetchPlay}
              className="inline-flex items-center gap-2 rounded-md bg-white px-7 py-2.5 text-base font-bold text-black transition-colors hover:bg-white/85"
            >
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="currentColor"
                aria-hidden
              >
                <path d="M6 4l14 8-14 8V4z" />
              </svg>
              {progress !== null ? "Resume" : "Play"}
            </a>
            <button
              type="button"
              onClick={toggleMyList}
              aria-label={inList ? "Remove from My List" : "Add to My List"}
              className="flex h-11 w-11 items-center justify-center rounded-full border-2 border-white/60 text-white transition-colors hover:border-white"
            >
              {inList ? (
                <svg
                  width="18"
                  height="18"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="3"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden
                >
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              ) : (
                <svg
                  width="18"
                  height="18"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  aria-hidden
                >
                  <line x1="12" y1="5" x2="12" y2="19" />
                  <line x1="5" y1="12" x2="19" y2="12" />
                </svg>
              )}
            </button>
            <button
              type="button"
              aria-label="Mark as liked"
              className="flex h-11 w-11 items-center justify-center rounded-full border-2 border-white/60 text-white transition-colors hover:border-white"
            >
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                aria-hidden
              >
                <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" />
              </svg>
            </button>
          </div>
        </div>
      </div>

      <div className="grid gap-8 px-10 pb-8 pt-6 lg:grid-cols-3">
        <div className="space-y-4 lg:col-span-2">
          <div className="flex flex-wrap items-center gap-3 text-sm text-white">
            {item.year && <span>{item.year}</span>}
            {isShow && seasons.length > 0 && (
              <span>
                {seasons.length} Season{seasons.length > 1 ? "s" : ""}
              </span>
            )}
            {item.duration && !isShow && (
              <span>{formatRuntime(item.duration)}</span>
            )}
            {item.contentRating && (
              <span className="rounded border border-white/40 px-1.5 py-0.5 text-xs font-medium">
                {item.contentRating}
              </span>
            )}
          </div>
          {item.summary && (
            <p className="text-base leading-relaxed text-white/95">
              {item.summary}
            </p>
          )}
        </div>
        <div className="space-y-3 text-sm">
          {topCast.length > 0 && (
            <div>
              <span className="text-white/60">Cast: </span>
              <span className="text-white">
                {topCast.map((c) => c.name).join(", ")}
                {extraCast && (
                  <>
                    , <span className="italic text-white/60">and more</span>
                  </>
                )}
              </span>
            </div>
          )}
          {item.genres && item.genres.length > 0 && (
            <div>
              <span className="text-white/60">Genres: </span>
              <span className="text-white">{item.genres.join(", ")}</span>
            </div>
          )}
          {item.directors && item.directors.length > 0 && !isShow && (
            <div>
              <span className="text-white/60">Director: </span>
              <span className="text-white">{item.directors.join(", ")}</span>
            </div>
          )}
          {item.rating !== undefined && (
            <div>
              <span className="text-white/60">Rating: </span>
              <span className="text-white">{item.rating.toFixed(1)} / 10</span>
            </div>
          )}
        </div>
      </div>

      {isShow && seasons.length > 0 && firstSeason && (
        <SeasonEpisodes
          seasons={seasons}
          initialEpisodes={initialEpisodes}
          initialSeasonKey={firstSeason.ratingKey}
        />
      )}

      {similar.length > 0 && <MoreLikeThis items={similar} />}
    </>
  );
}

function MoreLikeThis({ items }: { items: MediaItem[] }) {
  return (
    <section className="border-t border-white/10 px-10 py-8">
      <h2 className="mb-6 text-2xl font-medium">More Like This</h2>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {items.slice(0, 9).map((it) => (
          <SimilarCard key={it.ratingKey} item={it} />
        ))}
      </div>
    </section>
  );
}

function SimilarCard({ item }: { item: MediaItem }) {
  const img = plexImage(item.art ?? item.thumb, 480, 270);
  const label = displayTitle(item);
  return (
    <button
      type="button"
      onClick={() => openModal(item.ratingKey)}
      onMouseEnter={() => prefetchModalData(item.ratingKey)}
      onFocus={() => prefetchModalData(item.ratingKey)}
      className="group block w-full cursor-pointer overflow-hidden rounded-md bg-black text-left transition-colors hover:bg-white/5"
    >
      <div className="relative aspect-video">
        {img ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={img}
            alt={label}
            loading="lazy"
            className="h-full w-full object-cover"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-sm text-white/40">
            {label}
          </div>
        )}
        {item.duration && (
          <div className="absolute right-2 top-2 rounded bg-black/70 px-2 py-0.5 text-xs font-medium">
            {formatRuntime(item.duration)}
          </div>
        )}
      </div>
      <div className="p-3">
        <div className="mb-1.5 flex items-center gap-2 text-xs">
          {item.contentRating && (
            <span className="rounded border border-white/40 px-1.5 py-0.5 font-medium">
              {item.contentRating}
            </span>
          )}
          {item.year && <span className="text-white/85">{item.year}</span>}
        </div>
        <div className="line-clamp-2 text-sm font-medium">{label}</div>
      </div>
    </button>
  );
}
