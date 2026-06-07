"use client";

import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  episodes as episodesApi,
  playState as playStateApi,
} from "@/lib/chimpflix-api";
import { formatRuntime, type MediaItem } from "@/lib/chimpflix-types";
import { plexImage } from "@/lib/image";
import { TOAST_DISMISS_MS } from "@/lib/toast";
import { usePlayedThresholdPct } from "@/lib/server-config";
import { upcomingAirLabel } from "@/lib/relative-time";
import { MarkerEditor } from "./MarkerEditor";

export function SeasonEpisodes({
  seasons,
  initialEpisodes,
  initialSeasonKey,
  isOwner = false,
  targetEpisodeKey,
  onWatchStatsChange,
  bulkWatchVersion = 0,
  showPoster,
}: {
  seasons: MediaItem[];
  initialEpisodes: MediaItem[];
  initialSeasonKey: string;
  /// When true, render a small "Edit markers" button on each episode
  /// row. Owner-only because the underlying PUT replaces every
  /// manual marker on the file; misuse is destructive enough that
  /// it shouldn't be a co-editing surface.
  isOwner?: boolean;
  /// When the modal was opened with a specific episode in mind (e.g.
  /// clicking a Continue Watching tile for S3E5), this is its rating
  /// key. We swap to the matching season if needed and scroll the row
  /// into view so the user lands where they expected.
  targetEpisodeKey?: string;
  /// Called after per-episode / bulk watched toggles so the parent can
  /// refresh `detail.watch_stats` — without this the show-level "Mark
  /// all as watched" toggle stays in its pre-toggle state until the
  /// modal is closed and reopened.
  onWatchStatsChange?: () => void;
  /// Monotonically bumped by the parent after a show-level bulk
  /// watched toggle. Triggers a refetch of the currently selected
  /// season so the "Up Next" chip (computed from local `episodes`
  /// state) reflects the post-bulk watched flags instead of the
  /// stale pre-bulk array.
  bulkWatchVersion?: number;
  /// The show's poster (TMDB-relative path). Used as the thumbnail for
  /// PLACEHOLDER episodes (no downloaded file → no episode still) on the
  /// client refetch path, where the season list response carries only a
  /// null `thumb_path` and not the show's artwork. SSR rows already get
  /// this via `adaptEpisode`'s poster fallback; this keeps the two paths
  /// consistent so a placeholder shows the poster (Trakt-style) rather
  /// than an empty black box after the user switches seasons.
  showPoster?: string;
}) {
  const [selectedKey, setSelectedKey] = useState(initialSeasonKey);
  const [episodes, setEpisodes] = useState(initialEpisodes);
  const [loading, setLoading] = useState(false);
  // Ref to the row matching `targetEpisodeKey`, populated when it's in
  // the current season's render. Used by the scroll effect below.
  const targetRowRef = useRef<HTMLLIElement | null>(null);
  // Once we've scrolled to the target episode, don't keep re-scrolling
  // on subsequent renders (e.g. the parent re-rendering after a watched
  // toggle). The flag resets if the target changes.
  const scrolledToTargetRef = useRef<string | null>(null);
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

  // "Now" reference for the relative air-date labels ("Tomorrow", "In 2
  // weeks"). Snapshotted post-mount rather than read inline during render:
  // an inline Date.now() differs between the SSR pass and the client's
  // first paint, which would flag a hydration mismatch on any upcoming-
  // episode badge. Until this fills (first paint), nowMs is null and the
  // air-label is simply not rendered — the row shows its normal runtime,
  // exactly matching what the server emitted.
  const [nowMs, setNowMs] = useState<number | null>(null);
  useEffect(() => {
    // Post-hydration clock read — deliberately client-only so SSR and the
    // first client render agree (avoids a hydration mismatch). Same pattern
    // as Card.tsx.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setNowMs(Date.now());
  }, []);

  const changeSeason = useCallback(async function changeSeason(
    seasonKey: string,
  ) {
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
          air_date: number | null;
          duration_ms: number | null;
          thumb_path: string | null;
          // False = placeholder (no downloaded file). Optional/defaults
          // true so a season fetched before the backend grew the flag
          // never strips affordances off a real episode.
          has_file?: boolean;
          play_state: {
            position_ms: number;
            max_position_ms: number;
            watched?: boolean;
          } | null;
        }[];
      };
      if (!aliveRef.current || requestIdRef.current !== myRequestId) return;
      setEpisodes(
        data.episodes.map((e) => {
          // Placeholder = no downloaded file. It has no episode still, so
          // fall back to the show poster (Trakt-style), matching the SSR
          // adapter's behaviour. Real episodes keep their own still.
          const hasFile = e.has_file !== false;
          const thumb = hasFile
            ? (e.thumb_path ?? showPoster ?? undefined)
            : (showPoster ?? undefined);
          return {
            ratingKey: `e${e.id}`,
            key: `/episodes/${e.id}`,
            type: "episode",
            title: e.title,
            summary: e.summary ?? undefined,
            thumb,
            duration: e.duration_ms ?? undefined,
            airDate: e.air_date ?? undefined,
            // Furthest-watched drives the bar + "X min left" (resume still
            // uses position_ms server-side), so skipping around no longer
            // makes a finished episode look un-watched.
            viewOffset: e.play_state?.max_position_ms ?? undefined,
            watched: e.play_state?.watched ?? false,
            index: e.episode_number,
            hasFile,
          };
        }),
      );
    } finally {
      if (aliveRef.current && requestIdRef.current === myRequestId) {
        setLoading(false);
      }
    }
  }, [showPoster]);

  // Refetch the currently selected season after a show-level bulk
  // watched toggle (Mark all as watched / unwatched). Skips the
  // initial mount so we don't double-fetch on first render.
  const lastSeenBulkVersionRef = useRef(bulkWatchVersion);
  useEffect(() => {
    if (lastSeenBulkVersionRef.current === bulkWatchVersion) return;
    lastSeenBulkVersionRef.current = bulkWatchVersion;
    void changeSeason(selectedKey);
  }, [bulkWatchVersion, selectedKey, changeSeason]);

  // When the modal was opened with a target episode that lives in a
  // different season than `initialSeasonKey`, fetch it to discover its
  // season_id and switch. The scroll effect below picks up once the
  // right season's episodes have rendered.
  useEffect(() => {
    if (!targetEpisodeKey || !targetEpisodeKey.startsWith("e")) return;
    if (episodes.some((ep) => ep.ratingKey === targetEpisodeKey)) return;
    const epId = Number.parseInt(targetEpisodeKey.slice(1), 10);
    if (!Number.isFinite(epId)) return;
    let cancelled = false;
    (async () => {
      try {
        const detail = await episodesApi.get(epId);
        if (cancelled) return;
        const nextSeasonKey = `s${detail.season_id}`;
        if (nextSeasonKey === selectedKey) return;
        void changeSeason(nextSeasonKey);
      } catch {
        // Best-effort — leave the modal on the initial season.
      }
    })();
    return () => {
      cancelled = true;
    };
    // selectedKey + episodes are intentionally not deps: we only want
    // this resolution pass on a fresh target. The scroll effect below
    // handles the post-switch settle.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [targetEpisodeKey, changeSeason]);

  // Scroll the target row into view once the matching season is loaded.
  // The ref-based gate prevents re-scrolling on benign re-renders (a
  // watched toggle, marker edit, etc).
  useEffect(() => {
    if (!targetEpisodeKey) return;
    if (scrolledToTargetRef.current === targetEpisodeKey) return;
    if (!episodes.some((ep) => ep.ratingKey === targetEpisodeKey)) return;
    const id = requestAnimationFrame(() => {
      targetRowRef.current?.scrollIntoView({
        behavior: "smooth",
        block: "center",
      });
      scrolledToTargetRef.current = targetEpisodeKey;
    });
    return () => cancelAnimationFrame(id);
  }, [targetEpisodeKey, episodes]);

  const selectedSeason = seasons.find((s) => s.ratingKey === selectedKey);

  // Identify the "Up Next" episode within the current season — the row
  // the show-level Play button would actually start. Priority:
  //   1. The first in-progress episode (has viewOffset, not watched,
  //      and not past the server's "effectively watched" threshold —
  //      otherwise an episode the user finished but where the scrobble
  //      didn't fire would stay flagged forever).
  //   2. The first unwatched episode (also past-threshold-aware).
  //   3. None (season is fully watched).
  // Highlighted with a faint background tint + "Up Next" chip so the
  // user can tell at a glance where playback will resume.
  const thresholdPct = usePlayedThresholdPct();
  const isEffectivelyWatched = (ep: MediaItem): boolean => {
    if (ep.watched) return true;
    const pos = ep.viewOffset ?? 0;
    const dur = ep.duration ?? 0;
    if (dur <= 0 || pos <= 0) return false;
    return (pos / dur) * 100 >= thresholdPct;
  };
  // Placeholder episodes (no downloaded file) are never an "Up Next"
  // candidate — there's nothing to play. `playable()` gates the
  // in-progress / first-unwatched scans below so playback never resumes on
  // a file-less future episode.
  const playable = (ep: MediaItem): boolean => ep.hasFile !== false;
  const upNextIdx = (() => {
    const inProgress = episodes.findIndex(
      (ep) =>
        playable(ep) && !isEffectivelyWatched(ep) && (ep.viewOffset ?? 0) > 0,
    );
    if (inProgress !== -1) return inProgress;
    const firstUnwatched = episodes.findIndex(
      (ep) => playable(ep) && !isEffectivelyWatched(ep),
    );
    return firstUnwatched === -1 ? null : firstUnwatched;
  })();

  // Highest episode_number in the loaded season — the season finale.
  // Derived from the season's own episode list (which SeasonEpisodes
  // already holds) rather than any per-row flag, so it stays correct as
  // the user switches seasons. `ep.index` carries episode_number (set in
  // both the SSR adapter and the changeSeason refetch). A single-episode
  // season is both premiere and finale; we only label it "Finale" if it
  // has more than one episode so a one-off special isn't mislabelled.
  const maxEpisodeNumber = episodes.reduce(
    (max, ep) => Math.max(max, ep.index ?? 0),
    0,
  );

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
      onWatchStatsChange?.();
      setConfirmation(watched ? "Marked as watched" : "Marked as unwatched");
    } catch {
      // Revert on failure.
      setEpisodes((prev) =>
        prev.map((ep) =>
          ratingKeyToEpisodeId(ep.ratingKey) === episodeId
            ? { ...ep, watched: !watched }
            : ep,
        ),
      );
      setConfirmation("Couldn't update. Try again.");
    }
  }

  // Bulk "Mark season as watched / unwatched". If any episode is
  // unwatched the action is "mark all watched"; if all are watched
  // the action flips to "mark all unwatched". Same optimistic +
  // revert pattern, but the revert path snapshots the full list so a
  // partial-failure mid-loop doesn't leave the UI in a torn state.
  const [bulkBusy, setBulkBusy] = useState(false);
  // Ephemeral aria-live confirmation for screen readers. Sighted users
  // see the row update optimistically; SR users were getting no audible
  // feedback that the watched flip landed. Auto-clears after 3s so a
  // user who tabs back to the row a moment later doesn't hear stale
  // state announced again.
  const [confirmation, setConfirmation] = useState<string | null>(null);
  useEffect(() => {
    if (!confirmation) return;
    const t = window.setTimeout(() => setConfirmation(null), TOAST_DISMISS_MS);
    return () => window.clearTimeout(t);
  }, [confirmation]);
  async function bulkToggleSeason() {
    // Placeholder episodes (no file) can't be watched, so they're excluded
    // from both the "are they all watched?" check and the set of ids we
    // toggle. Without this, a season with any upcoming placeholder would
    // never read as fully-watched and we'd POST set_watched for file-less
    // rows the backend has no play-state for.
    const downloaded = episodes.filter((ep) => ep.hasFile !== false);
    if (bulkBusy || downloaded.length === 0) return;
    const allWatched = downloaded.every((ep) => ep.watched);
    const target = !allWatched;
    const ids = downloaded
      .map((ep) => ratingKeyToEpisodeId(ep.ratingKey))
      .filter((id): id is number => id != null);
    const snapshot = episodes;
    setBulkBusy(true);
    setEpisodes((prev) =>
      prev.map((ep) =>
        ep.hasFile !== false ? { ...ep, watched: target } : ep,
      ),
    );
    try {
      // Sequential loop to avoid issuing N parallel POST requests for large seasons.
      for (const id of ids) {
        await playStateApi.setWatched({ episode_id: id, watched: target });
      }
      onWatchStatsChange?.();
      setConfirmation(
        target
          ? "Season marked as watched"
          : "Season marked as unwatched",
      );
    } catch {
      setEpisodes(snapshot);
      setConfirmation("Couldn't update the season. Try again.");
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
      {/* Visually-hidden live region — SR announces "Marked as
          watched" etc. after each toggle. Sighted users get the
          optimistic row update; this is purely for accessibility. */}
      <span aria-live="polite" className="sr-only">
        {confirmation ?? ""}
      </span>
      <div className="mb-6 flex items-center justify-between gap-3">
        <h2 className="text-2xl font-medium">Episodes</h2>
        <div className="flex items-center gap-3">
          {/* Only shown when the season has at least one downloaded
              episode — a season made up entirely of upcoming placeholders
              has nothing to mark watched. The label reflects the
              downloaded episodes' state, ignoring file-less placeholders. */}
          {episodes.some((ep) => ep.hasFile !== false) && (
            <button
              type="button"
              onClick={() => void bulkToggleSeason()}
              disabled={bulkBusy}
              className="inline-flex items-center gap-1.5 rounded-md border border-white/20 px-3 py-2 text-sm text-white/80 transition-colors hover:border-white/40 hover:text-white disabled:opacity-50"
            >
              {episodes
                .filter((ep) => ep.hasFile !== false)
                .every((ep) => ep.watched)
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
          const remainingMs =
            ep.viewOffset && ep.duration && ep.viewOffset < ep.duration
              ? ep.duration - ep.viewOffset
              : null;
          const isUpNext = idx === upNextIdx;
          // PLACEHOLDER = an episode the metadata agent knows about but for
          // which no file has been downloaded (in-progress / future season).
          // These render Trakt-style: show poster, muted, no play / watched
          // / marker affordances, and a "Not downloaded · Airs <date>" note.
          // `hasFile` is undefined for non-episode/legacy sources → treated
          // as a real downloaded episode (renders exactly as before).
          const isPlaceholder = ep.hasFile === false;
          // Relative air label ("Today" / "Tomorrow" / "In 2 weeks"),
          // only for episodes that haven't aired yet. Null when the
          // episode has no air date, has already aired, or before the
          // post-mount nowMs snapshot lands — in which case the row keeps
          // its normal runtime display untouched.
          const airLabel =
            ep.airDate != null && nowMs != null
              ? upcomingAirLabel(ep.airDate, nowMs)
              : null;
          // Absolute air date ("Jun 5, 2026") for the placeholder
          // "Not downloaded · Airs <date>" affordance. Gated on the
          // post-mount nowMs snapshot so the locale/timezone-dependent
          // toLocaleDateString output can't differ between the SSR pass and
          // the client's first paint (which would flag a hydration mismatch).
          // `airDate` is stored at midnight UTC as a plain calendar date, so
          // it's formatted in UTC — formatting in local time would name the
          // wrong day (Jun 2 for a Jun 3 air date) for viewers west of UTC,
          // the same slip the calendar surfaces avoid (see relative-time.ts).
          const airDateStr =
            isPlaceholder && ep.airDate != null && nowMs != null
              ? new Date(ep.airDate).toLocaleDateString(undefined, {
                  month: "short",
                  day: "numeric",
                  year: "numeric",
                  timeZone: "UTC",
                })
              : null;
          // Season premiere = episode 1 (series premiere too if it's
          // also season 1; "Premiere" reads fine for both). Finale = the
          // highest episode_number in the season, but only when the
          // season has more than one episode so a lone special isn't
          // tagged. episode_number lives in ep.index. NOTE: maxEpisodeNumber
          // is computed over ALL episodes including placeholders, so an
          // undownloaded finale (e.g. E12) is correctly badged the Finale
          // even while only E1–E9 have files.
          const epNum = ep.index ?? 0;
          const isPremiere = epNum === 1;
          const isFinale = epNum > 1 && epNum === maxEpisodeNumber;

          // Shared inner content (thumbnail + metadata). The wrapper differs:
          // downloaded episodes are a <Link> to /watch; placeholders are a
          // plain <div> (nothing to play) with no hover/play affordances.
          const inner = (
            <>
                <div className="flex w-8 shrink-0 items-start pt-2 text-2xl font-medium text-white/60">
                  {idx + 1}
                </div>
                <div className="relative aspect-video w-44 shrink-0 overflow-hidden rounded bg-black">
                  {thumb && (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img
                      src={thumb}
                      alt=""
                      className={
                        isPlaceholder
                          ? "h-full w-full object-cover opacity-40"
                          : "h-full w-full object-cover transition-opacity group-hover:opacity-70"
                      }
                      loading="lazy"
                    />
                  )}
                  {/* Play overlay + progress bar are playback affordances —
                      suppressed for placeholders (no file to play). */}
                  {!isPlaceholder && (
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
                  )}
                  {!isPlaceholder && progress !== null && (
                    <div className="absolute inset-x-2 bottom-1.5 h-0.75 rounded-full bg-white/25">
                      <div
                        className="h-full rounded-full bg-(--color-accent)"
                        style={{ width: `${progress}%` }}
                      />
                    </div>
                  )}
                  {airLabel && (
                    <span className="absolute left-1.5 top-1.5 rounded bg-black/70 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-white backdrop-blur-sm">
                      {airLabel}
                    </span>
                  )}
                </div>
                <div className="flex-1">
                  <div className="mb-1 flex items-baseline justify-between gap-3">
                    <h3
                      className={`flex items-center gap-2 text-base font-medium ${
                        isPlaceholder ? "text-white/55" : ""
                      }`}
                    >
                      {ep.title}
                      {!isPlaceholder && ep.watched && (
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
                      {isUpNext && (
                        <span className="rounded-sm border border-accent/40 bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-accent">
                          Up Next
                        </span>
                      )}
                      {isPremiere && (
                        <span className="rounded-sm border border-white/20 bg-white/5 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-white/70">
                          Premiere
                        </span>
                      )}
                      {isFinale && (
                        <span className="inline-flex items-center gap-1 rounded-sm border border-accent/40 bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-accent">
                          <span
                            className="inline-block h-1.5 w-1.5 rounded-full bg-(--color-accent)"
                            aria-hidden
                          />
                          Finale
                        </span>
                      )}
                    </h3>
                    {ep.duration && (
                      <span className="shrink-0 text-sm text-white/60">
                        {remainingMs
                          ? `${formatRuntime(remainingMs)} left`
                          : formatRuntime(ep.duration)}
                      </span>
                    )}
                  </div>
                  {ep.summary && (
                    <p
                      className={`line-clamp-3 text-sm ${
                        isPlaceholder ? "text-white/45" : "text-white/70"
                      }`}
                    >
                      {ep.summary}
                    </p>
                  )}
                  {isPlaceholder ? (
                    // Placeholder affordance — no watched toggle / markers /
                    // play (not downloaded). A muted "Not downloaded" pill
                    // plus, when we know it, "Airs <date>", mirroring Trakt's
                    // greyed upcoming rows.
                    <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-white/45">
                      <span className="inline-flex items-center gap-1.5 rounded border border-white/10 px-2 py-0.5">
                        <svg
                          width="11"
                          height="11"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          aria-hidden
                        >
                          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                          <polyline points="7 10 12 15 17 10" />
                          <line x1="12" y1="15" x2="12" y2="3" />
                        </svg>
                        Not downloaded
                      </span>
                      {airDateStr && <span>Airs {airDateStr}</span>}
                    </div>
                  ) : (
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
                  )}
                </div>
            </>
          );

          return (
            <li
              key={ep.ratingKey}
              ref={ep.ratingKey === targetEpisodeKey ? targetRowRef : undefined}
            >
              {isPlaceholder ? (
                // Not a link — there's nothing to play. Plain row, no hover
                // highlight, slightly muted.
                <div className="-mx-3 flex gap-4 rounded-md px-3 py-5">
                  {inner}
                </div>
              ) : (
                <Link
                  href={`/watch/${ep.ratingKey}`}
                  className={`group -mx-3 flex gap-4 rounded-md px-3 py-5 transition-colors hover:bg-white/5 ${
                    isUpNext ? "bg-white/3" : ""
                  }`}
                >
                  {inner}
                </Link>
              )}
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
