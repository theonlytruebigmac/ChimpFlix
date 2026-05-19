"use client";

import Link from "next/link";
import {
  displayTitle,
  formatRuntime,
  type MediaItem,
} from "@/lib/chimpflix-types";
import { plexImage, plexSrcSet } from "@/lib/image";
import { openModal } from "@/lib/modal";
import { prefetchModalData } from "@/lib/modal-cache";
import { useMyListItem } from "@/lib/my-list";
import { prefetchPlay } from "@/lib/play-prefetch";
import { useRecentlyAddedDays } from "@/lib/server-config";

function recencyBadge(item: MediaItem, windowDays: number): string | null {
  // `windowDays = 0` is the explicit "no badge ever" setting.
  if (windowDays <= 0) return null;
  if (!item.addedAt) return null;
  const windowMs = windowDays * 24 * 60 * 60 * 1000;
  if (Date.now() - item.addedAt > windowMs) return null;
  // For TV shows where we know a new season just landed, surface that
  // over generic "Recently Added". (Season-add tracking lives in
  // `latestSeasonAt` once we wire it; until then any recently-added show
  // shows as "Recently Added".)
  if (item.type === "show" && item.childCount && item.childCount > 1) {
    return "New Season";
  }
  return "Recently Added";
}

export function Card({ item }: { item: MediaItem }) {
  const img = plexImage(item.art ?? item.thumb, 480, 270);
  const srcSet = plexSrcSet(item.art ?? item.thumb, 480, 270);
  const progress =
    item.viewOffset && item.duration
      ? Math.min(100, (item.viewOffset / item.duration) * 100)
      : null;
  const recentlyAddedDays = useRecentlyAddedDays();
  const badge = recencyBadge(item, recentlyAddedDays);

  const label = displayTitle(item);
  const modalKey =
    item.type === "episode" && item.grandparentRatingKey
      ? item.grandparentRatingKey
      : item.ratingKey;

  return (
    <div
      // Mobile shows ~2 cards across a 360px viewport; desktop keeps the
      // original 18rem (288px) sizing. Tailwind's `hover:` variant
      // already gates on `(hover: hover)` in v4 so the scale-up only
      // fires on devices with real hover — touch taps won't stick.
      className="group relative w-44 flex-none sm:w-56 md:w-72 hover:z-50"
      onMouseEnter={() => prefetchModalData(modalKey)}
      onFocus={() => prefetchModalData(modalKey)}
    >
      <div className="card-scaler origin-center transition-transform duration-200 ease-out delay-200 group-hover:scale-125">
        <div className="overflow-hidden rounded-md bg-(--color-surface) shadow-md group-hover:shadow-2xl">
          <button
            type="button"
            onClick={() => openModal(modalKey)}
            aria-label={label}
            className="block w-full cursor-pointer text-left"
          >
            <div className="relative aspect-video bg-black">
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
                the progress bar and any badges we add later.
              */}
              <div className="pointer-events-none absolute inset-x-0 top-0 bg-linear-to-b from-black/85 via-black/40 to-transparent pb-10 transition-opacity duration-150 delay-200 group-hover:opacity-0">
                <div className="line-clamp-2 px-3 pt-2.5 text-sm font-semibold leading-tight drop-shadow-lg">
                  {label}
                </div>
              </div>
              {badge && (
                <div className="pointer-events-none absolute bottom-2 left-0 z-10 select-none rounded-r-sm bg-(--color-accent) px-2 py-1 text-[0.7rem] font-bold uppercase leading-none tracking-wide text-white shadow-md transition-opacity duration-150 delay-200 group-hover:opacity-0">
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
              <HoverPanel item={item} modalKey={modalKey} label={label} />
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
  label,
}: {
  item: MediaItem;
  modalKey: string;
  label: string;
}) {
  const isShow = item.type === "show";
  const seasonCount = isShow ? item.childCount : undefined;
  const { inList, toggle: toggleMyList } = useMyListItem(modalKey);

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
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            toggleMyList();
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
        <CircleButton aria-label="Mark as liked">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
            <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" />
          </svg>
        </CircleButton>
        <CircleButton
          className="ml-auto"
          aria-label="More info"
          onClick={() => openModal(modalKey)}
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
        <span className="rounded border border-white/40 px-1.5 py-px text-[0.65rem] font-semibold tracking-wider">
          HD
        </span>
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
