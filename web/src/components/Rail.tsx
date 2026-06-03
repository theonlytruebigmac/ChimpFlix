import Link from "next/link";
import type { MediaItem } from "@/lib/chimpflix-types";
import { Card } from "./Card";
import { RailPrefetch } from "./RailPrefetch";
import { RailScroller } from "./RailScroller";

// Items most likely to be clicked from a given rail. We warm modal data for
// these in browser idle time so the first click feels instant. 2 is a
// compromise: more aggressive prefetching helps perceived speed but costs
// concurrent Plex calls per page.
const PREFETCH_COUNT = 2;

function modalKeyFor(it: MediaItem): string {
  return it.type === "episode" && it.grandparentRatingKey
    ? it.grandparentRatingKey
    : it.ratingKey;
}

export function Rail({
  title,
  items,
  href,
}: {
  title: string;
  items: MediaItem[];
  href?: string;
}) {
  if (items.length === 0) return null;
  const prefetchKeys = items.slice(0, PREFETCH_COUNT).map(modalKeyFor);
  return (
    // content-visibility:auto lets the browser skip layout, paint, and
    // event work for rails that are off-screen, then snap them in when
    // they come into view. contain-intrinsic-size reserves a placeholder
    // height so the scrollbar doesn't jump as rails materialize.
    // Combined with image lazy-loading, a 15-rail home page only spends
    // paint cycles on the 2-3 rails actually visible.
    <section
      className="zf-rise-in px-4 pb-1 pt-1 sm:px-8 md:px-12"
      style={{
        contentVisibility: "auto",
        containIntrinsicSize: "260px",
      }}
    >
      <h2 className="mb-3 text-lg font-semibold tracking-tight sm:text-xl md:text-[1.4rem]">
        {href ? (
          <Link
            href={href}
            className="group inline-flex items-center gap-1 transition-colors hover:text-(--color-accent)"
          >
            <span>{title}</span>
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
              className="opacity-0 transition-all duration-200 group-hover:translate-x-1 group-hover:opacity-100"
              aria-hidden
            >
              <polyline points="9 18 15 12 9 6" />
            </svg>
          </Link>
        ) : (
          title
        )}
      </h2>
      {/*
        Top padding is intentionally small to keep the title close to the
        cards (Netflix-tight). overflow-x: auto forces overflow-y to clip,
        so the hovered card's upward expansion crops slightly — acceptable
        trade-off for the spacing. Bottom padding keeps the hover panel
        visible.
      */}
      {/*
        first:/last: variants reach into Card's .card-scaler and override
        its origin so the edge cards expand inward instead of being clipped
        by the rail's overflow-x boundary. Middle cards keep origin-center.
      */}
      {/* `snap-x snap-mandatory` makes touch swipes land on a card
          rather than mid-card — feels closer to a native carousel.
          Disabled at md+ where there's no real swipe gesture and snap
          can interrupt mouse-wheel scrolling. */}
      <RailScroller className="-mx-1 flex snap-x snap-mandatory touch-pan-x gap-1.5 overflow-x-auto overscroll-x-contain px-1 pb-6 pt-2 scrollbar-none md:snap-none md:pb-20 [&::-webkit-scrollbar]:hidden">
        {items.map((it) => (
          <li
            key={it.ratingKey}
            className="snap-start flex-none [&:first-child_.card-scaler]:origin-top-left [&:last-child_.card-scaler]:origin-top-right"
          >
            <Card item={it} />
          </li>
        ))}
      </RailScroller>
      <RailPrefetch ratingKeys={prefetchKeys} />
    </section>
  );
}
