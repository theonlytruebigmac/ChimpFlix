"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import {
  episodes as episodesApi,
  playState as playStateApi,
} from "@/lib/chimpflix-api";
import { formatRuntime, type MediaItem } from "@/lib/chimpflix-types";
import { plexImage } from "@/lib/image";
import { MarkerEditor } from "./MarkerEditor";

export function SeasonEpisodes({
  seasons,
  initialEpisodes,
  initialSeasonKey,
  isOwner = false,
}: {
  seasons: MediaItem[];
  initialEpisodes: MediaItem[];
  initialSeasonKey: string;
  /// When true, render a small "Edit markers" button on each episode
  /// row. Owner-only because the underlying PUT replaces every
  /// manual marker on the file; misuse is destructive enough that
  /// it shouldn't be a co-editing surface.
  isOwner?: boolean;
}) {
  const [selectedKey, setSelectedKey] = useState(initialSeasonKey);
  const [episodes, setEpisodes] = useState(initialEpisodes);
  const [loading, setLoading] = useState(false);
  // Marker-editor state. We track the episode whose markers are being
  // edited (id + label for the header) plus the resolved primary
  // media_file_id. The id is fetched on click via episodes.get; the
  // season list response doesn't carry it. Wrapped in one object so
  // a re-open with a new episode atomically swaps all three.
  const [editTarget, setEditTarget] = useState<{
    episodeId: number;
    mediaFileId: number;
    label: string;
  } | null>(null);
  const [editLoadingId, setEditLoadingId] = useState<number | null>(null);
  const [editError, setEditError] = useState<string | null>(null);
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
          play_state: { position_ms: number; watched?: boolean } | null;
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
          watched: e.play_state?.watched ?? false,
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

  // Per-episode watched toggle. Optimistic — flips local state
  // first, then fires the server call; on failure we revert so the
  // UI doesn't lie about what the backend believes. setWatched also
  // emits the Trakt history push server-side when the user has Trakt
  // linked, so we don't need any client wiring for that.
  async function toggleEpisodeWatched(episodeId: number, watched: boolean) {
    setEpisodes((prev) =>
      prev.map((ep) =>
        ratingKeyToEpisodeId(ep.ratingKey) === episodeId
          ? { ...ep, watched }
          : ep,
      ),
    );
    try {
      await playStateApi.setWatched({ episode_id: episodeId, watched });
    } catch {
      // Revert on failure.
      setEpisodes((prev) =>
        prev.map((ep) =>
          ratingKeyToEpisodeId(ep.ratingKey) === episodeId
            ? { ...ep, watched: !watched }
            : ep,
        ),
      );
    }
  }

  // Bulk "Mark season as watched / unwatched". If any episode is
  // unwatched the action is "mark all watched"; if all are watched
  // the action flips to "mark all unwatched". Same optimistic +
  // revert pattern, but the revert path snapshots the full list so a
  // partial-failure mid-loop doesn't leave the UI in a torn state.
  const [bulkBusy, setBulkBusy] = useState(false);
  async function bulkToggleSeason() {
    if (bulkBusy || episodes.length === 0) return;
    const allWatched = episodes.every((ep) => ep.watched);
    const target = !allWatched;
    const ids = episodes
      .map((ep) => ratingKeyToEpisodeId(ep.ratingKey))
      .filter((id): id is number => id != null);
    const snapshot = episodes;
    setBulkBusy(true);
    setEpisodes((prev) => prev.map((ep) => ({ ...ep, watched: target })));
    try {
      await Promise.all(
        ids.map((id) =>
          playStateApi.setWatched({ episode_id: id, watched: target }),
        ),
      );
    } catch {
      setEpisodes(snapshot);
    } finally {
      setBulkBusy(false);
    }
  }

  // Click handler for the per-episode "Edit markers" button (owner
  // only). Resolves the episode's primary media file (the first
  // entry in the detail response) so we can open the editor — the
  // season list rows don't carry the file id directly.
  async function openMarkerEditor(episodeId: number, label: string) {
    setEditLoadingId(episodeId);
    setEditError(null);
    try {
      const detail = await episodesApi.get(episodeId);
      const primary = detail.files[0];
      if (!primary) {
        setEditError("Episode has no media file on disk.");
        return;
      }
      setEditTarget({ episodeId, mediaFileId: primary.id, label });
    } catch (e) {
      setEditError(e instanceof Error ? e.message : String(e));
    } finally {
      setEditLoadingId(null);
    }
  }

  return (
    <section className="border-t border-white/10 px-4 sm:px-8 md:px-12 py-8">
      <div className="mb-6 flex items-center justify-between gap-3">
        <h2 className="text-2xl font-medium">Episodes</h2>
        <div className="flex items-center gap-3">
          {episodes.length > 0 && (
            <button
              type="button"
              onClick={() => void bulkToggleSeason()}
              disabled={bulkBusy}
              className="inline-flex items-center gap-1.5 rounded-md border border-white/20 px-3 py-2 text-sm text-white/80 transition-colors hover:border-white/40 hover:text-white disabled:opacity-50"
            >
              {episodes.every((ep) => ep.watched)
                ? "Mark season as unwatched"
                : "Mark season as watched"}
            </button>
          )}
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
                    <h3 className="flex items-center gap-2 text-base font-medium">
                      {ep.title}
                      {ep.watched && (
                        <span
                          aria-label="Watched"
                          title="Watched"
                          className="inline-flex h-4 w-4 items-center justify-center rounded-full bg-(--color-accent) text-white"
                        >
                          <svg
                            width="10"
                            height="10"
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
                        </span>
                      )}
                    </h3>
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
                  <div className="mt-2 flex flex-wrap items-center gap-2">
                    <button
                      type="button"
                      onClick={(e) => {
                        // Stop the parent <Link> from intercepting
                        // the click and navigating to /watch.
                        e.preventDefault();
                        e.stopPropagation();
                        const id = ratingKeyToEpisodeId(ep.ratingKey);
                        if (id != null) {
                          void toggleEpisodeWatched(id, !ep.watched);
                        }
                      }}
                      className="inline-flex items-center gap-1.5 rounded border border-white/15 px-2 py-0.5 text-[11px] text-white/75 transition-colors hover:border-white/30 hover:text-white"
                    >
                      {ep.watched ? "Mark as unwatched" : "Mark as watched"}
                    </button>
                    {isOwner && (
                      <button
                        type="button"
                        onClick={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          const id = ratingKeyToEpisodeId(ep.ratingKey);
                          if (id != null) {
                            void openMarkerEditor(id, ep.title);
                          }
                        }}
                        disabled={
                          editLoadingId === ratingKeyToEpisodeId(ep.ratingKey)
                        }
                        className="inline-flex items-center gap-1.5 rounded border border-white/15 px-2 py-0.5 text-[11px] text-white/75 transition-colors hover:border-white/30 hover:text-white disabled:opacity-50"
                      >
                        {editLoadingId === ratingKeyToEpisodeId(ep.ratingKey)
                          ? "Loading…"
                          : "Edit markers"}
                      </button>
                    )}
                  </div>
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
      {editError && (
        <div className="mt-3 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {editError}
        </div>
      )}
      {editTarget && (
        <MarkerEditor
          mediaFileId={editTarget.mediaFileId}
          fileLabel={editTarget.label}
          open
          onClose={() => setEditTarget(null)}
        />
      )}
    </section>
  );
}

/// The season list uses "e<id>" prefixed rating keys to disambiguate
/// from movie ids. Strip the prefix to get the numeric episode id.
function ratingKeyToEpisodeId(ratingKey: string): number | null {
  if (!ratingKey.startsWith("e")) return null;
  const n = Number.parseInt(ratingKey.slice(1), 10);
  return Number.isFinite(n) ? n : null;
}
