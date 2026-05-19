"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useRouter } from "next/navigation";
import { brandNameUpper } from "@/lib/env";
import {
  displayTitle,
  formatRuntime,
  type MediaItem,
} from "@/lib/chimpflix-types";
import { plexImage } from "@/lib/image";
import { openModal } from "@/lib/modal";
import {
  getOrFetchModalData,
  prefetchModalData,
  type ModalData,
} from "@/lib/modal-cache";
import { useMyListItem } from "@/lib/my-list";
import Link from "next/link";
import {
  auth as authApi,
  collections as collectionsApi,
  items as itemsApi,
  libraries as librariesApi,
  playState as playStateApi,
  ratings as ratingsApi,
  tags as tagsApi,
  type Collection,
  type CollectionDetail,
  type Credit,
  type Extra,
  type ItemDetail,
  type ListedItem,
  type Review,
  type ReviewsSummary,
  type Tag,
  type User,
} from "@/lib/chimpflix-api";
import { EditMetadataDialog } from "./EditMetadataDialog";
import { FixMatchDialog } from "./FixMatchDialog";
import { prefetchPlay } from "@/lib/play-prefetch";
import { cancelPrewarm, prewarmFor } from "@/lib/prewarm";
import { detectClientCapabilities } from "@/lib/client-caps";
import { getPrefs } from "@/lib/prefs";
import { TitleModalShell } from "./TitleModalShell";
import { SeasonEpisodes } from "./SeasonEpisodes";
import { TrailerPlayer } from "./TrailerPlayer";

function useTrailer(item: MediaItem): string | null {
  const [videoId, setVideoId] = useState<string | null>(null);

  useEffect(() => {
    if (item.type !== "movie" && item.type !== "show") return;
    const id = Number.parseInt(item.ratingKey, 10);
    if (!Number.isFinite(id) || id <= 0) return;
    let cancelled = false;
    let timer: number | undefined;

    itemsApi
      .trailer(id)
      .then((data) => {
        if (cancelled || !data.video_id) return;
        // Brief delay before swapping the static art for the trailer so the
        // user has time to read the title and synopsis.
        timer = window.setTimeout(() => {
          if (!cancelled) setVideoId(data.video_id ?? null);
        }, 2000);
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [item.ratingKey, item.type]);

  return videoId;
}

/// Plays the community-curated theme song for a TV show when the modal
/// is open and no trailer is taking the audio channel. The plexapp
/// endpoint serves MP3s by tvdb_id; ~half of shows have a theme there
/// so we tolerate a 404 by silently doing nothing.
function useThemeMusic(tvdbId: number | null, enabled: boolean) {
  useEffect(() => {
    if (!tvdbId || !enabled) return;
    if (typeof Audio === "undefined") return;
    // Skip on touch-primary devices. Stacking an autoplay <audio> on
    // top of everything else the modal mounts (trailer iframe, preview
    // video, and then the full HLS player when the viewer hits Play)
    // overruns the mobile Chrome media stack and triggers an "Aw, snap"
    // renderer crash. Battery + data are also nice things to save on
    // phones. Desktop keeps the theme song — that's where it actually
    // adds to the browse experience.
    if (
      typeof window !== "undefined" &&
      window.matchMedia?.("(hover: none) and (pointer: coarse)").matches
    ) {
      return;
    }
    const audio = new Audio(`https://tvthemes.plexapp.com/${tvdbId}.mp3`);
    audio.loop = true;
    audio.volume = 0.35;
    audio.preload = "auto";
    const teardown = () => {
      audio.pause();
      audio.src = "";
    };
    audio.addEventListener("error", () => {
      teardown();
    });
    audio.play().catch(() => {
      // Autoplay policies; the user can still trigger via interaction.
      teardown();
    });
    return () => teardown();
  }, [tvdbId, enabled]);
}

export function TitleModalClient({ ratingKey }: { ratingKey: string }) {
  const [data, setData] = useState<ModalData | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    // Reset to the loading state when the modal switches to a different
    // ratingKey. The lint rule warns about synchronous setState in effects
    // but this is exactly the "respond to an input change" case where we
    // need to clear stale data before the async fetch resolves.
    // eslint-disable-next-line react-hooks/set-state-in-effect
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
  const [detail, setDetail] = useState<ItemDetail>(data.detail);
  const { item, seasons, initialEpisodes, similar, credits, extras } = data;
  const isShow = item.type === "show";
  const { inList, toggle: toggleMyList } = useMyListItem(item.ratingKey);
  const trailerVideoId = useTrailer(item);
  // Theme music: only attempt for TV shows with a tvdb_id, only when no
  // trailer is playing (the trailer brings its own audio). The
  // community-curated plexapp endpoint silently 404s for ~half of
  // shows, which we tolerate by hiding the <audio> on error.
  useThemeMusic(isShow ? detail.tvdb_id : null, !trailerVideoId);
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
  // True when any media file has at least one subtitle stream. Drives
  // the CC chip in the hero meta row so users know captions are
  // available before they hit Play.
  const hasSubtitles = detail.files.some((f) =>
    f.streams.some((s) => s.kind === "subtitle"),
  );

  // Hover-time session pre-warm. A 250 ms debounce keeps casual
  // pointer pass-throughs from spinning up ffmpeg — only deliberate
  // hovers (the kind that precede a click) make it through. Cancelling
  // on modal unmount tears down any orphan session if the user closes
  // the modal without clicking Play.
  const prewarmTimerRef = useRef<number | null>(null);
  const startPrewarm = () => {
    if (prewarmTimerRef.current !== null) return;
    prewarmTimerRef.current = window.setTimeout(() => {
      prewarmTimerRef.current = null;
      try {
        prewarmFor(
          item.ratingKey,
          {
            supported_video_codecs: detectClientCapabilities().video,
            supported_audio_codecs: detectClientCapabilities().audio,
            supported_containers: detectClientCapabilities().containers,
          },
          getPrefs().audioNormalize,
        );
      } catch {
        // Capability detection / pref read failures are best-effort;
        // the player's own createSession path is the source of truth.
      }
    }, 250);
  };
  const cancelPendingPrewarm = () => {
    if (prewarmTimerRef.current !== null) {
      window.clearTimeout(prewarmTimerRef.current);
      prewarmTimerRef.current = null;
    }
  };
  // The pre-warm session lives in a module-level cache that
  // outlives this component — when the user actually clicks Play
  // and the player adopts it, we don't want to cancel. So the
  // cleanup only fires `cancelPrewarm` if no consumption has
  // happened. The cache module's `consumePrewarm` zeros itself
  // out on hit, so `cancelPrewarm` becomes a no-op in that case.
  useEffect(() => {
    return () => {
      cancelPendingPrewarm();
      void cancelPrewarm();
    };
  }, []);

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
          {/*
            Netflix-style eyebrow: the brand mark in red, a thin vertical
            divider, then the kind. Sits just above the title so users
            can tell at a glance "movie" vs "series" vs "episode".
          */}
          <div className="mb-2 flex items-center gap-2 text-[0.7rem] font-bold uppercase tracking-[0.3em] drop-shadow">
            <span className="text-(--color-accent)">{brandNameUpper()}</span>
            <span className="h-3 w-px bg-white/40" aria-hidden />
            <span className="text-white/85">{isShow ? "Series" : "Film"}</span>
          </div>
          <div className="mb-5">
            {item.logo ? (
              // eslint-disable-next-line @next/next/no-img-element
              <img
                src={item.logo}
                alt={title}
                className="zf-fade-in max-h-44 max-w-md drop-shadow-2xl sm:max-h-56 sm:max-w-lg"
                style={{ objectFit: "contain", objectPosition: "left bottom" }}
              />
            ) : (
              <h1 className="max-w-3xl text-4xl font-black uppercase leading-[0.95] tracking-tight drop-shadow-lg sm:text-5xl">
                {title}
              </h1>
            )}
            {detail.original_title &&
              detail.original_title.trim() !== title.trim() && (
                <div
                  className="mt-2 max-w-3xl text-sm text-white/65 drop-shadow"
                  lang="ja"
                  title="Original title"
                >
                  {detail.original_title}
                </div>
              )}
          </div>

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
            <Link
              href={`/watch/${item.ratingKey}`}
              prefetch
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
            </Link>
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
            <WatchedToggle detail={detail} onUpdated={(next) => setDetail(next)} />
            <AdminActions
              detail={detail}
              onUpdated={(next) => setDetail(next)}
            />
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
            {/*
              Netflix-style HD + CC chips. HD is universal (we always
              transcode to H.264 ≥ 720p) so it's safe to show
              unconditionally. CC reflects whether ANY subtitle stream
              exists — embedded or external — so users know if captions
              are available before they start playing.
            */}
            <span className="rounded border border-white/40 px-1.5 py-0.5 text-[0.65rem] font-semibold tracking-wider">
              HD
            </span>
            {hasSubtitles && (
              <span className="rounded border border-white/40 px-1.5 py-0.5 text-[0.65rem] font-semibold tracking-wider">
                CC
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

      <FileInfoSection detail={detail} />
      <RatingBar itemId={detail.id} />
      <TagBar itemId={detail.id} />
      <ExternalLinks detail={detail} />
      {detail.collection_id != null && (
        <CollectionCard
          collectionId={detail.collection_id}
          currentItemId={detail.id}
        />
      )}
      {credits.length > 0 && <CastAndCrew credits={credits} />}
      {extras.length > 0 && <ExtrasRail extras={extras} />}
      <ReviewsSection itemRatingKey={item.ratingKey} initialSummary={data.reviews} />

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

// ─── File info / tech details ──────────────────────────────────────────────

// Plex-style strip of technical details for the title's primary file:
// resolution, codec, audio tracks, subtitle tracks, container, bit rate.
// For movies this is the single media file; for shows we show whatever
// counts the show has (episode-level files render in the per-episode UI).
function FileInfoSection({ detail }: { detail: ItemDetail }) {
  const file = detail.files[0];
  if (!file) return null;

  const audio = file.streams.filter((s) => s.kind === "audio");
  const subtitles = file.streams.filter((s) => s.kind === "subtitle");

  const resolution = (() => {
    // Map common pixel heights to user-friendly labels.
    const w = file.width;
    const h = file.height;
    if (!w || !h) return null;
    if (h >= 2000) return `${w}×${h} (4K)`;
    if (h >= 1000) return `${w}×${h} (1080p)`;
    if (h >= 700) return `${w}×${h} (720p)`;
    return `${w}×${h}`;
  })();

  const videoCodec = file.streams.find((s) => s.kind === "video")?.codec;

  function formatChannels(ch: number | null | undefined): string {
    if (!ch) return "";
    if (ch === 6) return "5.1";
    if (ch === 8) return "7.1";
    return `${ch}ch`;
  }

  return (
    <section className="border-t border-white/10 px-10 py-6">
      <h2 className="mb-4 text-sm font-semibold uppercase tracking-wider text-white/55">
        File Info
      </h2>
      <dl className="grid grid-cols-1 gap-x-8 gap-y-2 text-sm sm:grid-cols-2">
        {resolution && (
          <Row label="Video">
            {resolution}
            {videoCodec && (
              <span className="text-white/55"> · {videoCodec.toUpperCase()}</span>
            )}
            {file.hdr_format && (
              <span className="ml-2 rounded bg-white/10 px-1.5 py-0.5 text-xs font-medium uppercase tracking-wider text-white/85">
                {file.hdr_format}
              </span>
            )}
          </Row>
        )}
        {audio.length > 0 && (
          <Row label="Audio">
            {audio.map((s, i) => {
              const lang = s.language ?? "Unknown";
              const codec = s.codec ? s.codec.toUpperCase() : "";
              const ch = formatChannels(s.channels);
              const parts = [lang, codec, ch].filter(Boolean);
              return (
                <span key={i} className="mr-3">
                  {parts.join(" · ")}
                </span>
              );
            })}
          </Row>
        )}
        {subtitles.length > 0 && (
          <Row label="Subtitles">
            {subtitles
              .map((s) => s.language ?? "Unknown")
              .filter((v, i, a) => a.indexOf(v) === i)
              .join(", ")}
          </Row>
        )}
        {file.container && (
          <Row label="Container">{file.container.toUpperCase()}</Row>
        )}
        {file.bit_rate && (
          <Row label="Bitrate">{Math.round(file.bit_rate / 1000)} kbps</Row>
        )}
        {file.size_bytes > 0 && (
          <Row label="Size">{formatBytes(file.size_bytes)}</Row>
        )}
      </dl>
    </section>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-3">
      <dt className="w-24 shrink-0 text-xs uppercase tracking-wider text-white/45">
        {label}
      </dt>
      <dd className="min-w-0 flex-1 text-white/95">{children}</dd>
    </div>
  );
}

// ─── Watched toggle ────────────────────────────────────────────────────────

// Plex-style "Mark as watched / unwatched" button. Sits in the hero
// actions row next to Add-to-List. Optimistic: we update local state
// immediately and refetch the modal data so play_state, the Continue
// Watching rail, and the resume label reflect the new state.
function WatchedToggle({
  detail,
  onUpdated,
}: {
  detail: ItemDetail;
  onUpdated: (next: ItemDetail) => void;
}) {
  const watched = detail.play_state?.watched ?? false;
  const [busy, setBusy] = useState(false);

  // Shows aren't a single watch target; Plex marks the whole series via
  // recursive "all episodes" which we don't yet expose. Hide the toggle
  // for shows for now.
  if (detail.kind === "show") return null;

  async function toggle() {
    if (busy) return;
    setBusy(true);
    try {
      await playStateApi.setWatched({
        item_id: detail.id,
        watched: !watched,
      });
      // Refetch the detail so position_ms / view_count / watched are in
      // sync without us having to mirror the server's logic.
      const next = await itemsApi.get(detail.id);
      onUpdated(next);
    } catch {
      // Best-effort.
    } finally {
      setBusy(false);
    }
  }

  return (
    <button
      type="button"
      onClick={toggle}
      disabled={busy}
      aria-label={watched ? "Mark as unwatched" : "Mark as watched"}
      title={watched ? "Mark as unwatched" : "Mark as watched"}
      className={`flex h-11 w-11 items-center justify-center rounded-full border-2 transition-colors disabled:opacity-50 ${
        watched
          ? "border-(--color-accent) bg-(--color-accent) text-white"
          : "border-white/60 text-white hover:border-white"
      }`}
    >
      <svg
        width="18"
        height="18"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <polyline points="20 6 9 17 4 12" />
      </svg>
    </button>
  );
}

// ─── Collection card (movie franchises) ────────────────────────────────────

// Plex-style "Part of the X Collection" affordance. Hidden until we fetch
// the collection detail (which we do client-side to keep the modal route
// fast). Renders a small horizontal strip of sibling movies.
function CollectionCard({
  collectionId,
  currentItemId,
}: {
  collectionId: number;
  currentItemId: number;
}) {
  const [data, setData] = useState<CollectionDetail | null>(null);

  useEffect(() => {
    let cancelled = false;
    collectionsApi
      .get(collectionId)
      .then((d) => {
        if (!cancelled) setData(d);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [collectionId]);

  if (!data) return null;
  const siblings = data.items.filter((it) => it.id !== currentItemId);
  if (siblings.length === 0) return null;

  return (
    <section className="border-t border-white/10 px-10 py-8">
      <div className="mb-5 flex items-baseline justify-between gap-4">
        <div>
          <div className="text-xs uppercase tracking-wider text-white/45">
            Part of the
          </div>
          <Link
            href={`/collection/${data.id}`}
            className="text-2xl font-medium transition-colors hover:text-white"
          >
            {data.name}
          </Link>
        </div>
        <Link
          href={`/collection/${data.id}`}
          className="shrink-0 text-sm text-white/55 underline-offset-2 transition-colors hover:text-white hover:underline"
        >
          View all {data.item_count}
        </Link>
      </div>
      <div className="-mx-2 flex gap-3 overflow-x-auto px-2 pb-2">
        {siblings.slice(0, 8).map((it) => (
          <CollectionMemberTile key={it.id} item={it} />
        ))}
      </div>
    </section>
  );
}

function CollectionMemberTile({ item }: { item: ListedItem }) {
  const poster = item.poster_path ?? null;
  return (
    <button
      type="button"
      onClick={() => {
        // Switch the open modal to this sibling movie. The modal lives at
        // the app root and listens for openModal events.
        window.dispatchEvent(
          new CustomEvent("app:modal:open", { detail: String(item.id) }),
        );
      }}
      className="w-32 shrink-0 text-left transition-transform hover:scale-[1.03]"
    >
      <div className="aspect-2/3 overflow-hidden rounded bg-black/50">
        {poster ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={poster}
            alt={item.title}
            loading="lazy"
            className="h-full w-full object-cover"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center px-2 text-center text-xs text-white/45">
            {item.title}
          </div>
        )}
      </div>
      <div className="mt-1.5 line-clamp-2 text-xs font-medium">{item.title}</div>
      {item.year && (
        <div className="text-[10px] text-white/45">{item.year}</div>
      )}
    </button>
  );
}

// ─── External links ────────────────────────────────────────────────────────

// Small chip strip linking out to the title on every external service
// we've collected an id for. Hides itself when no ids are set so the
// border doesn't render a phantom row.
/// Per-user 1–10 rating widget. Stars rendered as small numbered
/// chips so the UI works without a star-glyph font. Hover previews
/// the rating; clicking commits, clicking the current rating clears.
/// Pushes to Trakt automatically when the user has linked their
/// account (the server-side handler fans the call out).
function RatingBar({ itemId }: { itemId: number }) {
  const [rating, setRating] = useState<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    ratingsApi
      .getItem(itemId)
      .then((r) => {
        if (!cancelled) {
          setRating(r.rating);
          setLoaded(true);
        }
      })
      .catch(() => {
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [itemId]);

  async function set(value: number) {
    if (busy) return;
    setBusy(true);
    try {
      if (rating === value) {
        // Clicking the current value clears the rating.
        await ratingsApi.deleteItem(itemId);
        setRating(null);
      } else {
        const r = await ratingsApi.putItem(itemId, value);
        setRating(r.rating ?? value);
      }
    } catch {
      // ignore — UI just stays as-is
    } finally {
      setBusy(false);
    }
  }

  if (!loaded) return null;
  const shown = hover ?? rating ?? 0;

  return (
    <div className="border-t border-white/10 px-10 py-4">
      <div className="flex items-center gap-3 text-xs">
        <span className="mr-1 text-white/45">Your rating:</span>
        <div
          className="flex gap-0.5"
          onMouseLeave={() => setHover(null)}
        >
          {Array.from({ length: 10 }, (_, i) => i + 1).map((n) => {
            const filled = n <= shown;
            return (
              <button
                key={n}
                type="button"
                onMouseEnter={() => setHover(n)}
                onClick={() => set(n)}
                disabled={busy}
                aria-label={`Rate ${n} out of 10`}
                className={`h-6 w-6 rounded text-[10px] font-semibold transition-colors ${
                  filled
                    ? "bg-(--color-accent) text-white"
                    : "bg-white/10 text-white/40 hover:bg-white/15"
                }`}
              >
                {n}
              </button>
            );
          })}
        </div>
        {rating !== null && (
          <span className="text-white/55">
            {rating}/10
            <span className="ml-2 text-white/40">click again to clear</span>
          </span>
        )}
      </div>
    </div>
  );
}

/// Operator-managed tag chips on the detail modal. Reads who the
/// current user is to decide whether to show the editing affordances
/// (add input + per-chip delete) — non-owners just see the chips.
function TagBar({ itemId }: { itemId: number }) {
  const [tags, setTags] = useState<Tag[] | null>(null);
  const [isOwner, setIsOwner] = useState(false);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [meRes, tagsRes] = await Promise.all([
          authApi.me().catch(() => null),
          tagsApi.forItem(itemId),
        ]);
        if (cancelled) return;
        setIsOwner(meRes?.user.role === "owner");
        setTags(tagsRes.tags);
      } catch {
        if (!cancelled) setTags([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [itemId]);

  async function addTag() {
    const name = draft.trim();
    if (!name || busy) return;
    setBusy(true);
    try {
      const tag = await tagsApi.add(itemId, name);
      setTags((current) =>
        current && current.some((t) => t.id === tag.id)
          ? current
          : [...(current ?? []), tag].sort((a, b) =>
              a.name.localeCompare(b.name),
            ),
      );
      setDraft("");
    } catch {
      // ignore — UI just stays as-is
    } finally {
      setBusy(false);
    }
  }

  async function removeTag(tagId: number) {
    setBusy(true);
    try {
      await tagsApi.remove(itemId, tagId);
      setTags((current) => (current ?? []).filter((t) => t.id !== tagId));
    } catch {
      // ignore
    } finally {
      setBusy(false);
    }
  }

  // Hide the row entirely when there are no tags AND the viewer
  // can't add any — no point taking up vertical space.
  if (tags === null) return null;
  if (tags.length === 0 && !isOwner) return null;

  return (
    <div className="border-t border-white/10 px-10 py-4">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="mr-1 text-white/45">Tags:</span>
        {tags.map((t) => (
          <span
            key={t.id}
            className="inline-flex items-center gap-1 rounded-full border border-white/15 bg-white/5 px-3 py-1"
          >
            {t.name}
            {isOwner && (
              <button
                type="button"
                onClick={() => removeTag(t.id)}
                aria-label={`Remove tag ${t.name}`}
                className="text-white/40 transition-colors hover:text-white"
              >
                ×
              </button>
            )}
          </span>
        ))}
        {isOwner && (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              addTag();
            }}
            className="inline-flex items-center gap-1"
          >
            <input
              type="text"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="add tag…"
              maxLength={64}
              className="w-28 rounded-full border border-dashed border-white/15 bg-transparent px-3 py-1 outline-none placeholder-white/30 focus:border-white/40"
            />
          </form>
        )}
      </div>
    </div>
  );
}

function ExternalLinks({ detail }: { detail: ItemDetail }) {
  const isShow = detail.kind === "show";
  const tmdbUrl = detail.tmdb_id
    ? `https://www.themoviedb.org/${isShow ? "tv" : "movie"}/${detail.tmdb_id}`
    : null;
  const imdbUrl = detail.imdb_id
    ? `https://www.imdb.com/title/${detail.imdb_id}/`
    : null;
  // Letterboxd accepts either tmdb_id or imdb_id directly in the URL —
  // the redirect server resolves either to the canonical film page.
  // Skip for shows (Letterboxd is movies-only).
  const letterboxdUrl =
    !isShow && (detail.tmdb_id || detail.imdb_id)
      ? `https://letterboxd.com/${detail.tmdb_id ? `tmdb/${detail.tmdb_id}` : `imdb/${detail.imdb_id}`}/`
      : null;
  const tvdbUrl = detail.tvdb_id
    ? `https://thetvdb.com/?tab=${isShow ? "series" : "movie"}&id=${detail.tvdb_id}`
    : null;
  const anilistUrl = detail.anilist_id
    ? `https://anilist.co/anime/${detail.anilist_id}`
    : null;
  if (!tmdbUrl && !imdbUrl && !letterboxdUrl && !tvdbUrl && !anilistUrl)
    return null;
  return (
    <div className="border-t border-white/10 px-10 py-4">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="mr-1 text-white/45">More about this title:</span>
        {tmdbUrl && <LinkChip href={tmdbUrl} label="TMDB" />}
        {imdbUrl && <LinkChip href={imdbUrl} label="IMDb" />}
        {letterboxdUrl && (
          <LinkChip href={letterboxdUrl} label="Letterboxd" />
        )}
        {tvdbUrl && <LinkChip href={tvdbUrl} label="TheTVDB" />}
        {anilistUrl && <LinkChip href={anilistUrl} label="AniList" />}
      </div>
    </div>
  );
}

function LinkChip({ href, label }: { href: string; label: string }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="inline-flex items-center gap-1.5 rounded-full border border-white/15 bg-white/5 px-3 py-1 font-medium transition-colors hover:border-white/40 hover:bg-white/10"
    >
      {label}
      <svg
        width="10"
        height="10"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <path d="M7 17 17 7" />
        <path d="M7 7h10v10" />
      </svg>
    </a>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

// ─── Admin actions (owner-only) ────────────────────────────────────────────

function AdminActions({
  detail,
  onUpdated,
}: {
  detail: ItemDetail;
  onUpdated: (next: ItemDetail) => void;
}) {
  const router = useRouter();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [user, setUser] = useState<User | null>(null);
  const [open, setOpen] = useState(false);
  const [showEdit, setShowEdit] = useState(false);
  const [showMatch, setShowMatch] = useState(false);
  const [showDelete, setShowDelete] = useState(false);
  const [showAddToCollection, setShowAddToCollection] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [detectingMarkers, setDetectingMarkers] = useState(false);
  /// Brief one-shot toast for fire-and-forget admin actions
  /// (currently the marker-detect kickoff). The backend returns 202
  /// with "queued: N" and runs the work in the background, so the
  /// menu shows a quick acknowledgement and disappears.
  const [actionToast, setActionToast] = useState<string | null>(null);
  // Handle for the 4s auto-clear timer. Tracked so a rapid second
  // action cancels the first action's pending clear — without this,
  // the second toast would be wiped 4s after the *first* trigger
  // instead of 4s after itself.
  const actionToastTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (actionToastTimerRef.current !== null) {
        window.clearTimeout(actionToastTimerRef.current);
        actionToastTimerRef.current = null;
      }
    };
  }, []);
  const showActionToast = useCallback((msg: string | null, autoClearMs = 4000) => {
    setActionToast(msg);
    if (actionToastTimerRef.current !== null) {
      window.clearTimeout(actionToastTimerRef.current);
      actionToastTimerRef.current = null;
    }
    if (msg !== null && autoClearMs > 0) {
      actionToastTimerRef.current = window.setTimeout(() => {
        actionToastTimerRef.current = null;
        setActionToast(null);
      }, autoClearMs);
    }
  }, []);
  // Whether the OWNING library has `allow_media_deletion = true`.
  // Fetched once on mount (per modal open). null = unknown / not yet
  // loaded; the Delete menu item only renders when true so the
  // operator doesn't see a destructive option that will server-reject.
  const [allowDelete, setAllowDelete] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    authApi
      .me()
      .then((res) => {
        if (!cancelled) setUser(res.user);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    // Skip the libraries fetch for non-owners — they can't act on the
    // result anyway and the endpoint may not be reachable.
    if (!user || user.role !== "owner") return;
    let cancelled = false;
    librariesApi
      .list()
      .then((r) => {
        if (cancelled) return;
        const lib = r.libraries.find((l) => l.id === detail.library_id);
        setAllowDelete(lib?.allow_media_deletion ?? false);
      })
      .catch(() => {
        if (!cancelled) setAllowDelete(false);
      });
    return () => {
      cancelled = true;
    };
  }, [user, detail.library_id]);

  if (!user || user.role !== "owner") return null;

  async function refresh() {
    if (refreshing) return;
    setRefreshing(true);
    setOpen(false);
    try {
      const next = await itemsApi.refresh(detail.id);
      onUpdated(next);
    } catch {
      // Best-effort; user can re-trigger.
    } finally {
      setRefreshing(false);
    }
  }

  async function detectMarkers() {
    if (detectingMarkers) return;
    setDetectingMarkers(true);
    setOpen(false);
    try {
      const { queued } = await itemsApi.detectMarkers(detail.id);
      showActionToast(
        queued === 0
          ? "No files to scan."
          : `Marker detection queued for ${queued} file${queued === 1 ? "" : "s"}.`,
      );
    } catch (e) {
      showActionToast(
        e instanceof Error ? `Failed: ${e.message}` : `Failed: ${String(e)}`,
      );
    } finally {
      setDetectingMarkers(false);
    }
  }

  function onDeleted(report: { items_purged: number }) {
    // If the cascade swept the item itself, no point in keeping the
    // modal open against a now-dead id — pop the URL back to the
    // page that opened us and let the rails re-fetch.
    if (report.items_purged > 0) {
      router.back();
    }
    // Otherwise (cascade didn't reach the item — partial show
    // delete), close the delete dialog but leave the modal so the
    // operator can decide what to do next.
  }

  return (
    <>
      <div className="relative">
        <button
          ref={triggerRef}
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-label="Admin actions"
          aria-haspopup="menu"
          aria-expanded={open}
          className="flex h-11 w-11 items-center justify-center rounded-full border-2 border-white/60 text-white transition-colors hover:border-white"
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="currentColor"
            aria-hidden
          >
            <circle cx="5" cy="12" r="2" />
            <circle cx="12" cy="12" r="2" />
            <circle cx="19" cy="12" r="2" />
          </svg>
        </button>
        {actionToast && (
          <div className="absolute right-0 top-full z-30 mt-2 w-64 rounded-md border border-white/15 bg-(--color-surface) px-3 py-2 text-xs text-white/80 shadow-2xl">
            {actionToast}
          </div>
        )}
      </div>
      <AdminActionsMenu
        open={open}
        anchorRef={triggerRef}
        onClose={() => setOpen(false)}
        items={[
          {
            kind: "item",
            label: "Edit Metadata…",
            icon: PencilIcon,
            onClick: () => {
              setOpen(false);
              setShowEdit(true);
            },
          },
          {
            kind: "item",
            label: "Fix Match…",
            icon: TargetIcon,
            onClick: () => {
              setOpen(false);
              setShowMatch(true);
            },
          },
          {
            kind: "item",
            label: refreshing ? "Refreshing…" : "Refresh metadata",
            icon: RefreshIcon,
            disabled: refreshing,
            onClick: refresh,
          },
          {
            kind: "item",
            label: detectingMarkers ? "Detecting…" : "Detect markers",
            icon: WaveIcon,
            disabled: detectingMarkers,
            onClick: detectMarkers,
          },
          {
            kind: "item",
            label: "Add to collection…",
            icon: FolderPlusIcon,
            onClick: () => {
              setOpen(false);
              setShowAddToCollection(true);
            },
          },
          ...(allowDelete
            ? ([
                { kind: "separator" as const },
                {
                  kind: "item" as const,
                  label: "Delete from disk…",
                  icon: TrashIcon,
                  destructive: true,
                  onClick: () => {
                    setOpen(false);
                    setShowDelete(true);
                  },
                },
              ] as const)
            : []),
        ]}
      />
      {showEdit && (
        <EditMetadataDialog
          detail={detail}
          onClose={() => setShowEdit(false)}
          onSaved={(next) => onUpdated(next)}
        />
      )}
      {showMatch && (
        <FixMatchDialog
          detail={detail}
          onClose={() => setShowMatch(false)}
          onApplied={(next) => onUpdated(next)}
        />
      )}
      {showDelete && (
        <DeleteMediaDialog
          detail={detail}
          onClose={() => setShowDelete(false)}
          onDeleted={onDeleted}
        />
      )}
      {showAddToCollection && (
        <AddToCollectionDialog
          itemId={detail.id}
          itemTitle={detail.title}
          onClose={() => setShowAddToCollection(false)}
        />
      )}
    </>
  );
}

/// Mini dialog opened from the item modal's admin menu. Lists existing
/// manual collections with one-click "Add" buttons, plus an inline
/// quick-create. Auto collections aren't shown — they're TMDB-driven
/// and rejected server-side anyway.
function AddToCollectionDialog({
  itemId,
  itemTitle,
  onClose,
}: {
  itemId: number;
  itemTitle: string;
  onClose: () => void;
}) {
  const [manualCollections, setManualCollections] = useState<Collection[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const r = await collectionsApi.list();
      setManualCollections(r.collections.filter((c) => c.kind === "manual"));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    collectionsApi
      .list()
      .then((r) => {
        if (cancelled) return;
        setManualCollections(r.collections.filter((c) => c.kind === "manual"));
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function addTo(collectionId: number, name: string) {
    setBusyId(collectionId);
    setError(null);
    try {
      const r = await collectionsApi.addItems(collectionId, [itemId]);
      setToast(
        r.inserted > 0
          ? `Added to “${name}”.`
          : `Already in “${name}”.`,
      );
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  async function createAndAdd(e: React.FormEvent) {
    e.preventDefault();
    const name = newName.trim();
    if (!name || creating) return;
    setCreating(true);
    setError(null);
    try {
      const { id } = await collectionsApi.create({ name });
      await collectionsApi.addItems(id, [itemId]);
      setToast(`Created “${name}” and added this item.`);
      setNewName("");
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  }

  // Portal: same reason as EditMetadata/FixMatch — the parent TitleModal
  // card's `zfModalIn` transform animation establishes a containing
  // block for fixed descendants, so without the portal this dialog
  // opens wherever the modal card is on screen instead of centered.
  if (typeof document === "undefined") return null;
  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-full max-w-md rounded-lg border border-white/15 bg-neutral-950 p-6 shadow-2xl space-y-4">
        <div className="flex items-baseline justify-between gap-2">
          <h2 className="text-lg font-semibold">Add to collection</h2>
          <button
            type="button"
            onClick={onClose}
            className="text-xs text-white/55 hover:text-white"
          >
            Close
          </button>
        </div>
        <p className="text-xs text-white/55">
          Add <span className="font-medium text-white/80">{itemTitle}</span>{" "}
          to one or more manual collections.
        </p>

        {error && (
          <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
            {error}
          </div>
        )}
        {toast && (
          <div className="rounded-md border border-white/15 bg-white/5 px-3 py-2 text-xs text-white/80">
            {toast}
          </div>
        )}

        {loading ? (
          <div className="text-xs text-white/50">Loading…</div>
        ) : manualCollections.length === 0 ? (
          <div className="rounded border border-dashed border-white/15 bg-white/2 px-3 py-4 text-center text-xs text-white/50">
            No manual collections yet. Create one below.
          </div>
        ) : (
          <ul className="max-h-60 divide-y divide-white/5 overflow-y-auto rounded border border-white/10">
            {manualCollections.map((c) => (
              <li
                key={c.id}
                className="flex items-center gap-2 px-3 py-2 text-sm"
              >
                <span className="grow truncate">{c.name}</span>
                <span className="shrink-0 text-xs tabular-nums text-white/50">
                  {c.item_count}
                </span>
                <button
                  type="button"
                  onClick={() => addTo(c.id, c.name)}
                  disabled={busyId === c.id}
                  className="rounded border border-white/15 px-2 py-0.5 text-xs text-white/80 hover:bg-white/5 disabled:opacity-50"
                >
                  {busyId === c.id ? "Adding…" : "Add"}
                </button>
              </li>
            ))}
          </ul>
        )}

        <form onSubmit={createAndAdd} className="flex items-end gap-2">
          <div className="grow">
            <label className="mb-1 block text-xs font-medium text-white/80">
              Create new
            </label>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              maxLength={200}
              placeholder="Collection name"
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm outline-none focus:border-white/30"
            />
          </div>
          <button
            type="submit"
            disabled={!newName.trim() || creating}
            className="rounded-md bg-red-500 px-3 py-1.5 text-sm font-semibold text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
          >
            {creating ? "Creating…" : "Create & add"}
          </button>
        </form>
      </div>
    </div>,
    document.body,
  );
}

/// Confirmation dialog for the hard-delete-from-disk path. Shows the
/// item name + file count, requires the operator to type the title
/// (matching Plex's destructive-action pattern), and displays the
/// resulting cascade summary on completion. No retry — failure
/// surfaces the error and the operator can dismiss and try again.
function DeleteMediaDialog({
  detail,
  onClose,
  onDeleted,
}: {
  detail: ItemDetail;
  onClose: () => void;
  onDeleted: (report: { items_purged: number }) => void;
}) {
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<{
    files_deleted: number;
    episodes_purged: number;
    seasons_purged: number;
    items_purged: number;
    paths: string[];
  } | null>(null);

  // For movies we have a file count locally; for shows we know how
  // many seasons but the per-episode file count is paged behind
  // season-detail fetches. "All files in this show" is honest
  // without pretending we know an exact number.
  const fileSummary =
    detail.kind === "movie"
      ? `${detail.files.length} file${detail.files.length === 1 ? "" : "s"}`
      : `every episode file across ${detail.seasons.length} season${detail.seasons.length === 1 ? "" : "s"}`;

  // Require the operator to type the title — same pattern Plex uses
  // for destructive bulk actions. Case-insensitive trim-match keeps
  // it ergonomic but still deliberate.
  const confirmed =
    confirmation.trim().toLowerCase() === detail.title.toLowerCase();

  async function performDelete() {
    if (!confirmed || busy || done) return;
    setBusy(true);
    setError(null);
    try {
      const r = await itemsApi.deleteMedia(detail.id);
      setDone(r);
      // Defer the parent callback so the operator can see the
      // result summary before the modal pops away.
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  function dismiss() {
    if (done) onDeleted({ items_purged: done.items_purged });
    onClose();
  }

  // Portal: same reason as the other in-modal dialogs.
  if (typeof document === "undefined") return null;
  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget) dismiss();
      }}
    >
      <div className="w-full max-w-md rounded-lg border border-red-500/30 bg-neutral-950 p-6 shadow-2xl">
        <h2 className="text-lg font-semibold text-red-300">
          Delete from disk
        </h2>
        {done ? (
          <>
            <p className="mt-3 text-sm text-white/80">
              Deleted{" "}
              <span className="font-medium">
                {done.files_deleted} file{done.files_deleted === 1 ? "" : "s"}
              </span>
              {done.episodes_purged > 0 && (
                <>, {done.episodes_purged} episode{done.episodes_purged === 1 ? "" : "s"}</>
              )}
              {done.seasons_purged > 0 && (
                <>, {done.seasons_purged} season{done.seasons_purged === 1 ? "" : "s"}</>
              )}
              {done.items_purged > 0 && (
                <>, {done.items_purged} item{done.items_purged === 1 ? "" : "s"}</>
              )}
              .
            </p>
            {done.paths.length > 0 && (
              <div className="mt-3 max-h-40 overflow-y-auto rounded border border-white/10 bg-black/40 p-2 font-mono text-[11px] text-white/55">
                {done.paths.map((p) => (
                  <div key={p} className="truncate">{p}</div>
                ))}
              </div>
            )}
            <div className="mt-5 flex justify-end">
              <button
                type="button"
                onClick={dismiss}
                className="rounded-md bg-white/10 px-4 py-2 text-sm font-semibold text-white hover:bg-white/20"
              >
                Close
              </button>
            </div>
          </>
        ) : (
          <>
            <p className="mt-3 text-sm text-white/80">
              Permanently delete <span className="font-medium">{detail.title}</span>{" "}
              ({fileSummary}) from disk. The media file
              {detail.files.length === 1 ? "" : "s"}, episodes, seasons, and
              orphan rows are removed immediately — no grace window, no undo.
            </p>
            <p className="mt-3 text-xs text-white/55">
              Type{" "}
              <span className="rounded bg-white/10 px-1 py-0.5 font-mono text-white/80">
                {detail.title}
              </span>{" "}
              to confirm.
            </p>
            <input
              type="text"
              value={confirmation}
              onChange={(e) => setConfirmation(e.target.value)}
              placeholder={detail.title}
              className="mt-2 w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-red-500/60"
              autoFocus
            />
            {error && (
              <div className="mt-3 rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
                {error}
              </div>
            )}
            <div className="mt-5 flex justify-end gap-2">
              <button
                type="button"
                onClick={onClose}
                disabled={busy}
                className="rounded-md border border-white/15 px-4 py-2 text-sm text-white/80 hover:bg-white/5 disabled:opacity-50"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={performDelete}
                disabled={!confirmed || busy}
                className="rounded-md bg-red-600 px-4 py-2 text-sm font-semibold text-white hover:bg-red-700 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
              >
                {busy ? "Deleting…" : "Delete permanently"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>,
    document.body,
  );
}

// ─── Admin actions menu (portal) ───────────────────────────────────────────
//
// Rendered via `createPortal` to document.body so the dropdown isn't
// clipped by the modal card's `overflow-hidden` (needed for rounded
// corners) or the hero image container's (needed for image cropping).
// Positioned `fixed`, anchored to the trigger's bottom-right via
// `getBoundingClientRect()` on open. Closes on outside click, Escape,
// and on window resize / modal scroll (re-opening will reposition).
//
// Item list is data-driven (Plex-style) so adding new actions later is
// just an array push — icons live below as small inline SVG components.

type AdminMenuItem =
  | { kind: "item"; label: string; icon: React.ComponentType; onClick: () => void; disabled?: boolean; destructive?: boolean }
  | { kind: "separator" };

function AdminActionsMenu({
  open,
  anchorRef,
  onClose,
  items,
}: {
  open: boolean;
  anchorRef: React.RefObject<HTMLButtonElement | null>;
  onClose: () => void;
  items: ReadonlyArray<AdminMenuItem>;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  // Anchor position computed from the trigger's bounding rect. `null`
  // before the first open (no SSR mismatch — portal mounts client-side).
  const [coords, setCoords] = useState<{ top: number; right: number } | null>(null);

  useEffect(() => {
    if (!open) return;
    const button = anchorRef.current;
    if (!button) return;
    const update = () => {
      const r = button.getBoundingClientRect();
      setCoords({
        top: r.bottom + 8,
        right: Math.max(8, window.innerWidth - r.right),
      });
    };
    update();
    // Reposition on window resize. Modal scroll fires on the backdrop
    // container; we listen with capture so we catch it regardless of
    // which scroll container moved.
    const onScroll = () => onClose();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [open, anchorRef, onClose]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (menuRef.current?.contains(target)) return;
      if (anchorRef.current?.contains(target)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, anchorRef, onClose]);

  if (!open || !coords) return null;
  if (typeof document === "undefined") return null;

  return createPortal(
    <div
      ref={menuRef}
      role="menu"
      style={{ top: coords.top, right: coords.right }}
      className="fixed z-70 w-56 overflow-hidden rounded-md border border-white/10 bg-(--color-surface) py-1 shadow-2xl ring-1 ring-black/40"
    >
      {items.map((it, i) => {
        if (it.kind === "separator") {
          return <div key={`sep-${i}`} className="my-1 h-px bg-white/8" aria-hidden />;
        }
        const Icon = it.icon;
        return (
          <button
            key={it.label}
            type="button"
            role="menuitem"
            onClick={it.onClick}
            disabled={it.disabled}
            className={`flex w-full items-center gap-3 px-3 py-2 text-left text-sm transition-colors disabled:opacity-50 ${
              it.destructive
                ? "text-red-300 hover:bg-red-500/15"
                : "text-white/90 hover:bg-white/8"
            }`}
          >
            <span className="flex h-4 w-4 shrink-0 items-center justify-center text-white/60">
              <Icon />
            </span>
            <span className="flex-1 truncate">{it.label}</span>
          </button>
        );
      })}
    </div>,
    document.body,
  );
}

// ─── Menu icons ───────────────────────────────────────────────────────────
// 16px stroke icons in the lucide style. Inline so the menu doesn't pull
// in an icon library for six glyphs.

const IconBase = ({ children }: { children: React.ReactNode }) => (
  <svg
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden
  >
    {children}
  </svg>
);

const PencilIcon = () => (
  <IconBase>
    <path d="M12 20h9" />
    <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
  </IconBase>
);
const TargetIcon = () => (
  <IconBase>
    <circle cx="12" cy="12" r="10" />
    <circle cx="12" cy="12" r="6" />
    <circle cx="12" cy="12" r="2" />
  </IconBase>
);
const RefreshIcon = () => (
  <IconBase>
    <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
    <path d="M21 3v5h-5" />
    <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
    <path d="M3 21v-5h5" />
  </IconBase>
);
const WaveIcon = () => (
  <IconBase>
    <path d="M2 12h2l3-9 4 18 3-12 3 6 2-3h3" />
  </IconBase>
);
const FolderPlusIcon = () => (
  <IconBase>
    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
    <line x1="12" y1="11" x2="12" y2="17" />
    <line x1="9" y1="14" x2="15" y2="14" />
  </IconBase>
);
const TrashIcon = () => (
  <IconBase>
    <polyline points="3 6 5 6 21 6" />
    <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
    <path d="M10 11v6" />
    <path d="M14 11v6" />
    <path d="M9 6V4a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2" />
  </IconBase>
);

// ─── Cast & Crew ───────────────────────────────────────────────────────────

function CastAndCrew({ credits }: { credits: Credit[] }) {
  // Split cast (acting) from crew (director/writer/producer). Cast is
  // shown with character names and headshots, Plex-style.
  const cast = credits.filter((c) => c.role_kind === "cast");
  return (
    <section className="border-t border-white/10 px-10 py-8">
      <h2 className="mb-6 text-2xl font-medium">Cast &amp; Crew</h2>
      <div className="-mx-2 flex gap-2 overflow-x-auto px-2 pb-2">
        {cast.map((c) => (
          <CastTile key={c.id} credit={c} />
        ))}
      </div>
    </section>
  );
}

function CastTile({ credit }: { credit: Credit }) {
  const photo = credit.person.photo_url ?? null;
  return (
    <div className="w-32 shrink-0 text-center">
      <div className="relative mx-auto h-28 w-28 overflow-hidden rounded-full bg-white/5">
        {photo ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={photo}
            alt={credit.person.name}
            loading="lazy"
            className="h-full w-full object-cover"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-2xl font-bold text-white/40">
            {credit.person.name
              .split(" ")
              .map((p) => p[0])
              .slice(0, 2)
              .join("")}
          </div>
        )}
      </div>
      <div className="mt-2 line-clamp-2 text-sm font-medium text-white">
        {credit.person.name}
      </div>
      {credit.character_name && (
        <div className="line-clamp-2 text-xs text-white/55">
          {credit.character_name}
        </div>
      )}
    </div>
  );
}

// ─── Extras (trailers, featurettes, BTS) ───────────────────────────────────

function ExtrasRail({ extras }: { extras: Extra[] }) {
  return (
    <section className="border-t border-white/10 px-10 py-8">
      <h2 className="mb-6 text-2xl font-medium">Extras</h2>
      <div className="-mx-2 flex gap-3 overflow-x-auto px-2 pb-2">
        {extras.map((e) => (
          <ExtraTile key={e.id} extra={e} />
        ))}
      </div>
    </section>
  );
}

function ExtraTile({ extra }: { extra: Extra }) {
  const href =
    extra.source === "youtube"
      ? `https://www.youtube.com/watch?v=${extra.source_id}`
      : "#";
  const kindLabel = (() => {
    switch (extra.kind) {
      case "trailer":
        return "Trailer";
      case "teaser":
        return "Teaser";
      case "featurette":
        return "Featurette";
      case "behind_the_scenes":
        return "Behind the Scenes";
      case "clip":
        return "Clip";
      case "deleted_scene":
        return "Deleted Scene";
      default:
        return extra.kind;
    }
  })();
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="group block w-72 shrink-0 overflow-hidden rounded-md bg-black/40 transition-colors hover:bg-white/5"
    >
      <div className="relative aspect-video bg-black">
        {extra.thumb_url && (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={extra.thumb_url}
            alt=""
            loading="lazy"
            className="h-full w-full object-cover"
          />
        )}
        <div className="absolute inset-0 flex items-center justify-center bg-black/0 transition-colors group-hover:bg-black/40">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-white/95 text-black opacity-0 transition-opacity group-hover:opacity-100">
            <svg
              width="20"
              height="20"
              viewBox="0 0 24 24"
              fill="currentColor"
              aria-hidden
            >
              <path d="M6 4l14 8-14 8V4z" />
            </svg>
          </div>
        </div>
      </div>
      <div className="p-3">
        <div className="text-xs uppercase tracking-wider text-white/55">
          {kindLabel}
        </div>
        <div className="mt-1 line-clamp-2 text-sm font-medium">{extra.title}</div>
      </div>
    </a>
  );
}

// ─── Reviews ───────────────────────────────────────────────────────────────

// Read-only "Top reviews" section. Reviews are pulled from the metadata
// provider (TMDB to start) at enrichment time and served from our DB —
// no in-app authoring.
function ReviewsSection({
  itemRatingKey,
  initialSummary,
}: {
  itemRatingKey: string;
  initialSummary: ReviewsSummary;
}) {
  const itemId = Number.parseInt(itemRatingKey, 10);
  const [reviews, setReviews] = useState<Review[] | null>(null);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  useEffect(() => {
    if (!Number.isFinite(itemId) || itemId <= 0) return;
    let cancelled = false;
    itemsApi
      .listReviews(itemId)
      .then((res) => {
        if (!cancelled) setReviews(res.reviews);
      })
      .catch(() => {
        if (!cancelled) setReviews([]);
      });
    return () => {
      cancelled = true;
    };
  }, [itemId]);

  // Hide the section entirely when there's nothing to show — an empty
  // "Top Reviews" block would just be visual noise.
  if (reviews !== null && reviews.length === 0) return null;

  // Cap visible reviews so the modal doesn't grow unbounded.
  const visible = (reviews ?? []).slice(0, 6);

  function toggle(id: number) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  return (
    <section className="border-t border-white/10 px-10 py-8">
      <div className="mb-6 flex items-baseline gap-4">
        <h2 className="text-2xl font-medium">Top Reviews</h2>
        {initialSummary.count > 0 && (
          <span className="text-sm text-white/55">
            {initialSummary.average !== null && (
              <>
                <span className="text-(--color-accent) font-semibold">
                  {initialSummary.average.toFixed(1)}
                </span>{" "}
                <span className="text-white/40">/ 10</span> ·{" "}
              </>
            )}
            {initialSummary.count}{" "}
            {initialSummary.count === 1 ? "review" : "reviews"}
          </span>
        )}
      </div>

      <ul className="space-y-3">
        {visible.map((r) => {
          const isExpanded = expanded.has(r.id);
          const body = r.body ?? "";
          const truncated = body.length > 360;
          const display =
            !truncated || isExpanded ? body : `${body.slice(0, 360)}…`;
          return (
            <li
              key={r.id}
              className="rounded-md border border-white/5 bg-white/2 p-4"
            >
              <div className="mb-2 flex items-center gap-3">
                <div className="h-9 w-9 shrink-0 overflow-hidden rounded-full bg-white/10">
                  {r.avatar_url ? (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img
                      src={r.avatar_url}
                      alt=""
                      loading="lazy"
                      className="h-full w-full object-cover"
                    />
                  ) : (
                    <div className="flex h-full w-full items-center justify-center text-sm font-semibold text-white/55">
                      {r.author[0]?.toUpperCase() ?? "?"}
                    </div>
                  )}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-baseline gap-3">
                    <span className="truncate font-semibold">{r.author}</span>
                    {r.rating !== null && (
                      <span className="shrink-0 text-sm font-semibold text-(--color-accent)">
                        {r.rating} / 10
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-white/45">
                    {new Date(r.created_at).toLocaleDateString()}
                    {r.source === "tmdb" && (
                      <span className="ml-2 text-white/30">via TMDB</span>
                    )}
                  </div>
                </div>
              </div>
              {body && (
                <p className="whitespace-pre-line text-sm leading-relaxed text-white/85">
                  {display}
                  {truncated && (
                    <button
                      type="button"
                      onClick={() => toggle(r.id)}
                      className="ml-1 text-white/55 underline-offset-2 transition-colors hover:text-white hover:underline"
                    >
                      {isExpanded ? "Show less" : "Show more"}
                    </button>
                  )}
                </p>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
