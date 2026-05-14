"use client";

import { useEffect, useState } from "react";
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
  playState as playStateApi,
  type CollectionDetail,
  type Credit,
  type Extra,
  type ItemDetail,
  type ListedItem,
  type Review,
  type ReviewsSummary,
  type User,
} from "@/lib/chimpflix-api";
import { EditMetadataDialog } from "./EditMetadataDialog";
import { FixMatchDialog } from "./FixMatchDialog";
import { prefetchPlay } from "@/lib/play-prefetch";
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
      <div className="aspect-[2/3] overflow-hidden rounded bg-black/50">
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

// Small chip strip linking out to the title on IMDb / TMDB. We only have
// these ids from TMDB enrichment (and TVMaze fallback for imdb_id), so the
// bar hides itself when no ids are set.
function ExternalLinks({ detail }: { detail: ItemDetail }) {
  const isShow = detail.kind === "show";
  const tmdbUrl = detail.tmdb_id
    ? `https://www.themoviedb.org/${isShow ? "tv" : "movie"}/${detail.tmdb_id}`
    : null;
  const imdbUrl = detail.imdb_id
    ? `https://www.imdb.com/title/${detail.imdb_id}/`
    : null;
  if (!tmdbUrl && !imdbUrl) return null;
  return (
    <div className="border-t border-white/10 px-10 py-4">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="mr-1 text-white/45">More about this title:</span>
        {tmdbUrl && (
          <LinkChip href={tmdbUrl} label="TMDB" />
        )}
        {imdbUrl && <LinkChip href={imdbUrl} label="IMDb" />}
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
  const [user, setUser] = useState<User | null>(null);
  const [open, setOpen] = useState(false);
  const [showEdit, setShowEdit] = useState(false);
  const [showMatch, setShowMatch] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

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

  return (
    <>
      <div className="relative">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-label="Admin actions"
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
        {open && (
          <div
            role="menu"
            className="absolute right-0 top-full z-30 mt-2 w-48 overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl"
          >
            <MenuItem
              onClick={() => {
                setOpen(false);
                setShowEdit(true);
              }}
            >
              Edit Metadata…
            </MenuItem>
            <MenuItem
              onClick={() => {
                setOpen(false);
                setShowMatch(true);
              }}
            >
              Fix Match…
            </MenuItem>
            <MenuItem onClick={refresh} disabled={refreshing}>
              {refreshing ? "Refreshing…" : "Refresh metadata"}
            </MenuItem>
          </div>
        )}
      </div>
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
    </>
  );
}

function MenuItem({
  children,
  onClick,
  disabled,
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      role="menuitem"
      className="block w-full px-4 py-2 text-left text-sm transition-colors hover:bg-white/10 disabled:opacity-50"
    >
      {children}
    </button>
  );
}

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
