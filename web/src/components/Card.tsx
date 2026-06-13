"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import {
  displayTitle,
  formatRuntime,
  qualityChipLabel,
  type MediaItem,
} from "@/lib/chimpflix-types";
import { plexImage, plexSrcSet } from "@/lib/image";
import { TOAST_DISMISS_MS } from "@/lib/toast";
import { openModal } from "@/lib/modal";
import { prefetchModalData } from "@/lib/modal-cache";
import { useMyListItem } from "@/lib/my-list";
import { useItemLike } from "@/lib/likes";
import { prefetchPlay } from "@/lib/play-prefetch";
import { useRecentlyAddedDays } from "@/lib/server-config";

function recencyBadge(
  item: MediaItem,
  windowDays: number,
  now: number,
): string | null {
  // `windowDays = 0` is the explicit "no badge ever" setting.
  if (windowDays <= 0) return null;
  if (!item.addedAt) return null;
  const windowMs = windowDays * 24 * 60 * 60 * 1000;
  if (now - item.addedAt > windowMs) return null;
  // For TV shows where we know a new season just landed, surface that
  // over generic "Recently Added". (Season-add tracking lives in
  // `latestSeasonAt` once we wire it; until then any recently-added show
  // shows as "Recently Added".)
  if (item.type === "show" && item.childCount && item.childCount > 1) {
    return "New Season";
  }
  return "Recently Added";
}

export function Card({
  item,
  variant = "backdrop",
}: {
  item: MediaItem;
  /// "backdrop" (default) renders the 16:9 backdrop the home rails use.
  /// "poster" renders a portrait 2:3 poster for Netflix-style Top 10
  /// rails where the giant numeral sits behind a narrow tile.
  variant?: "backdrop" | "poster";
}) {
  const isPoster = variant === "poster";
  const imgW = isPoster ? 240 : 480;
  const imgH = isPoster ? 360 : 270;
  const imgSrcPath = isPoster
    ? (item.thumb ?? item.art)
    : (item.art ?? item.thumb);
  const img = plexImage(imgSrcPath, imgW, imgH);
  const srcSet = plexSrcSet(imgSrcPath, imgW, imgH);
  const progress =
    item.viewOffset && item.duration
      ? Math.min(100, (item.viewOffset / item.duration) * 100)
      : null;
  const recentlyAddedDays = useRecentlyAddedDays();
  // Compute the recency badge after hydration. The badge depends on
  // `Date.now()` and the server's clock is N ms ahead of the browser's
  // by render time, so doing it in render lets the server emit
  // "Recently Added" while the client emits null (or vice versa) — a
  // textual hydration mismatch that triggers React error #418 and, on
  // mobile Chrome, an "Aw, snap" renderer crash. `now` starts undefined
  // so initial SSR + first client render agree (no badge); the effect
  // then fills it in.
  const [now, setNow] = useState<number | null>(null);
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setNow(Date.now());
  }, []);
  const badge = now === null ? null : recencyBadge(item, recentlyAddedDays, now);

  const label = displayTitle(item);
  // For episodes the modal target is the parent show (the modal renders
  // the show detail), but we hand the episode rating key along as a
  // hint so the modal lands on the right season and scrolls to the row
  // that was clicked — Continue Watching → S3E5 should not drop the
  // user on S1E1 anymore.
  const isEpisodeCard =
    item.type === "episode" && item.grandparentRatingKey != null;
  const modalKey = isEpisodeCard
    ? (item.grandparentRatingKey as string)
    : item.ratingKey;
  const episodeKeyHint = isEpisodeCard ? item.ratingKey : undefined;

  return (
    <div
      // Mobile shows ~2 cards across a 360px viewport; desktop keeps the
      // original 18rem (288px) sizing. Tailwind's `hover:` variant
      // already gates on `(hover: hover)` in v4 so the scale-up only
      // fires on devices with real hover — touch taps won't stick.
      // `focus-within` mirrors the hover state when the inner button
      // is focused via keyboard, so the user can see which card is
      // active without relying on the small default browser outline.
      className={`group relative flex-none has-focus-visible:z-50 hover:z-50 ${
        isPoster ? "w-28 sm:w-32 md:w-40" : "w-44 sm:w-56 md:w-72"
      }`}
      onMouseEnter={() => prefetchModalData(modalKey)}
      onFocus={() => prefetchModalData(modalKey)}
    >
      <div className="card-scaler origin-top transition-transform duration-200 ease-out delay-200 group-hover:scale-110 group-has-focus-visible:scale-110">
        <div className="overflow-hidden rounded-md bg-(--color-surface) shadow-md group-hover:shadow-2xl group-has-focus-visible:shadow-2xl group-has-focus-visible:ring-2 group-has-focus-visible:ring-accent group-has-focus-visible:ring-offset-2 group-has-focus-visible:ring-offset-background">
          <button
            type="button"
            onClick={() => openModal(modalKey, episodeKeyHint)}
            aria-label={label}
            // Suppress the default browser outline on the inner
            // button — the outer wrapper renders an accent ring
            // via `group-has-[:focus-visible]:ring-2` instead,
            // which is sized to the entire card.
            className="block w-full cursor-pointer text-left focus:outline-none focus-visible:outline-none"
          >
            <div
              className={`relative bg-black ${
                isPoster ? "aspect-2/3" : "aspect-video"
              }`}
            >
              {img ? (
                // eslint-disable-next-line @next/next/no-img-element
                <img
                  src={img}
                  srcSet={srcSet}
                  alt=""
                  loading="lazy"
                  decoding="async"
                  className="h-full w-full object-cover"
                />
              ) : (
                <div className="flex h-full w-full items-center justify-center text-sm text-white/40">
                  {label}
                </div>
              )}
              {/*
                Title overlay. Plex backdrop art usually doesn't bake in the
                title the way Netflix's marketing art does, so we add a
                gradient + text so viewers can identify the title at rest.
                Anchored top-left so the bottom of the card stays clean for
                the progress bar and any badges we add later. TMDB posters
                already include the title baked in, so we skip the overlay
                in poster variant to avoid double-titling.
              */}
              {!isPoster && (
                <div className="pointer-events-none absolute inset-x-0 top-0 bg-linear-to-b from-black/85 via-black/40 to-transparent pb-10 transition-opacity duration-200 delay-200 group-hover:opacity-0">
                  <div className="line-clamp-2 px-3 pt-2.5 text-sm font-semibold leading-tight drop-shadow-lg">
                    {label}
                  </div>
                </div>
              )}
              {badge && (
                // Badge stays visible through the hover state — the
                // information ("Recently Added" / "New Season") is
                // exactly what helps the user decide whether to click,
                // so fading it out at the moment of intent was the
                // wrong default.
                <div className="pointer-events-none absolute bottom-2 left-0 z-10 select-none rounded-r-sm bg-(--color-accent) px-2 py-1 text-[0.7rem] font-bold uppercase leading-none tracking-wide text-white shadow-md">
                  {badge}
                </div>
              )}
              {progress !== null && (
                <div className="absolute inset-x-0 bottom-0 h-0.75 bg-white/25">
                  <div
                    className="h-full bg-(--color-accent)"
                    style={{ width: `${progress}%` }}
                  />
                </div>
              )}
            </div>
          </button>

          <div className="grid grid-rows-[0fr] transition-[grid-template-rows] duration-200 ease-out delay-200 group-hover:grid-rows-[1fr]">
            <div className="overflow-hidden">
              <HoverPanel
                item={item}
                modalKey={modalKey}
                episodeKeyHint={episodeKeyHint}
                label={label}
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function HoverPanel({
  item,
  modalKey,
  episodeKeyHint,
  label,
}: {
  item: MediaItem;
  modalKey: string;
  episodeKeyHint?: string;
  label: string;
}) {
  const isShow = item.type === "show";
  const seasonCount = isShow ? item.childCount : undefined;
  const { inList, toggle: toggleMyList } = useMyListItem(modalKey);
  const { liked, toggle: toggleLike } = useItemLike(modalKey);

  // Ephemeral aria-live confirmation for screen readers. The icon
  // swap (filled ↔ outline) is enough for sighted users, but SR users
  // were getting nothing audible when these binary toggles flipped.
  // Auto-clears after 3s so revisiting the row doesn't re-announce.
  const [confirmation, setConfirmation] = useState<string | null>(null);
  useEffect(() => {
    if (!confirmation) return;
    const t = window.setTimeout(() => setConfirmation(null), TOAST_DISMISS_MS);
    return () => window.clearTimeout(t);
  }, [confirmation]);

  return (
    <div className="space-y-2 bg-(--color-surface) px-3 py-3">
      {/*
        Title — Netflix bakes the title into the card image art so they
        omit it from the hover panel, but we don't generate per-title
        logo art so we keep a thin one-liner here. Sized smaller than
        the meta below so it stays a label, not a heading.
      */}
      <div className="line-clamp-1 text-[0.78rem] font-semibold text-white">
        {label}
      </div>
      {/*
        Button row, Netflix order: filled Play (white) → outlined +
        (My List) → outlined thumbs-up (like) → flex-spacer → outlined
        chevron (More info). Same 7-w-7 size across the row keeps the
        rhythm tight; the Play button is the only filled one so the
        eye lands there first.
      */}
      <div className="flex items-center gap-1.5">
        <Link
          href={`/watch/${item.ratingKey}`}
          aria-label="Play"
          onMouseEnter={prefetchPlay}
          onFocus={prefetchPlay}
          className="flex h-7 w-7 items-center justify-center rounded-full bg-white text-black transition-colors hover:bg-white/85"
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
            <path d="M6 4l14 8-14 8V4z" />
          </svg>
        </Link>
        <CircleButton
          aria-label={inList ? "Remove from My List" : "Add to My List"}
          aria-pressed={inList}
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            const next = !inList;
            toggleMyList();
            setConfirmation(next ? "Added to My List" : "Removed from My List");
          }}
        >
          {inList ? (
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
              <polyline points="20 6 9 17 4 12" />
            </svg>
          ) : (
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" aria-hidden>
              <line x1="12" y1="5" x2="12" y2="19" />
              <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
          )}
        </CircleButton>
        <CircleButton
          aria-label={liked ? "Remove your like" : "I like this"}
          aria-pressed={liked}
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            const next = !liked;
            toggleLike();
            setConfirmation(next ? "Added to your ratings" : "Removed from your ratings");
          }}
        >
          <svg
            width="13"
            height="13"
            viewBox="0 0 24 24"
            fill={liked ? "currentColor" : "none"}
            stroke="currentColor"
            strokeWidth="2"
            aria-hidden
          >
            <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" />
          </svg>
        </CircleButton>
        <CircleButton
          className="ml-auto"
          aria-label="More info"
          onClick={() => openModal(modalKey, episodeKeyHint)}
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" aria-hidden>
            <polyline points="6 9 12 15 18 9" />
          </svg>
        </CircleButton>
      </div>

      {/*
        Meta row, Netflix order: maturity chip (with thin outline),
        runtime / season count, "HD" chip on the right. Sits tight
        against the buttons.
      */}
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[0.7rem] text-white/90">
        {item.contentRating && (
          <span className="rounded border border-white/40 px-1.5 py-px font-medium">
            {item.contentRating}
          </span>
        )}
        {seasonCount !== undefined && seasonCount > 0 ? (
          <span>{seasonCount} Season{seasonCount > 1 ? "s" : ""}</span>
        ) : null}
        {!isShow && item.duration && <span>{formatRuntime(item.duration)}</span>}
        {(() => {
          const q = qualityChipLabel(item);
          return q ? (
            <span className="rounded border border-white/40 px-1.5 py-px text-[0.65rem] font-semibold tracking-wider">
              {q}
            </span>
          ) : null;
        })()}
      </div>

      {/*
        Mood / genre line — 3 items max, separated by dots. Netflix
        uses curated mood tags ("Suspenseful · Witty · Family-Friendly");
        we fall back to genres because we don't generate mood tags
        locally.
      */}
      {item.genres && item.genres.length > 0 && (
        <div className="flex flex-wrap items-center gap-x-1.5 text-[0.7rem] text-white/85">
          {item.genres.slice(0, 3).map((g, i) => (
            <span key={g} className="flex items-center gap-1.5">
              {i > 0 && <span className="text-white/35">•</span>}
              {g}
            </span>
          ))}
        </div>
      )}
      <span aria-live="polite" className="sr-only">
        {confirmation ?? ""}
      </span>
    </div>
  );
}

/// Small round button used throughout the hover panel. White outline,
/// transparent fill, hover brightens the border. Sized 7×7 so the
/// row fits without crowding the filled Play button.
function CircleButton({
  children,
  className = "",
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      {...props}
      className={`flex h-7 w-7 items-center justify-center rounded-full border border-white/40 text-white transition-colors hover:border-white ${className}`}
    >
      {children}
    </button>
  );
}
