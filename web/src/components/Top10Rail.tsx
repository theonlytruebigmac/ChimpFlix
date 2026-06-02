"use client";

import Link from "next/link";
import type { MediaItem } from "@/lib/chimpflix-types";
import { Card } from "./Card";
import { RailScroller } from "./RailScroller";

/// Netflix-style "Top 10" rail: each entry is a portrait poster with a
/// giant outlined rank glyph sitting behind it. The number uses CSS
/// text stroke (-webkit-text-stroke) so it reads as outline-only
/// without needing per-character SVGs. Falls back to a heavy filled
/// number in browsers without text-stroke support, which is still
/// recognizably the Netflix design.
///
/// Scroll behavior matches the regular `Rail`: horizontal overflow,
/// touch snap-mandatory (md+ disables snap so a mouse wheel isn't
/// captured by the carousel). The optional `href` adds the same
/// chevron-on-hover title link the other rails use.
export function Top10Rail({
  title,
  items,
  href,
}: {
  title: string;
  items: Array<{ rank: number; item: MediaItem }>;
  href?: string;
}) {
  if (items.length === 0) return null;
  return (
    <section
      className="zf-rise-in px-4 pb-1 pt-1 sm:px-8 md:px-12"
      style={{ contentVisibility: "auto", containIntrinsicSize: "320px" }}
    >
      <h2 className="mb-3 flex items-baseline gap-2 text-lg font-semibold tracking-tight sm:text-xl md:text-[1.4rem]">
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
          <span>{title}</span>
        )}
        <span className="text-xs font-medium uppercase tracking-[0.18em] text-white/45">
          Top 10
        </span>
      </h2>
      <RailScroller className="-mx-1 flex snap-x snap-mandatory touch-pan-x gap-1.5 overflow-x-auto overscroll-x-contain px-1 pb-12 pt-2 scrollbar-none md:snap-none [&::-webkit-scrollbar]:hidden">
        {items.map(({ rank, item }) => (
          <li
            key={item.ratingKey}
            className="snap-start flex-none [&:first-child_.card-scaler]:origin-left [&:last-child_.card-scaler]:origin-right"
          >
            <Top10Entry rank={rank} item={item} />
          </li>
        ))}
      </RailScroller>
    </section>
  );
}

function Top10Entry({ rank, item }: { rank: number; item: MediaItem }) {
  // 10 is two-glyph, so it needs more horizontal real estate behind the
  // poster than 1-9. The glyph sits to the left of the poster and the
  // poster overlaps the right side of the glyph (-ml-* pulls it in).
  // Sizes are tuned to a 2:3 poster at the Card poster-variant widths
  // (w-28 sm:w-32 md:w-40), so the numeral height roughly matches the
  // poster height across breakpoints.
  const isTwoDigit = rank >= 10;
  return (
    <div className="relative flex items-stretch">
      <div
        aria-hidden
        className={`pointer-events-none relative shrink-0 select-none font-black leading-none ${
          isTwoDigit ? "w-32 sm:w-40 md:w-52" : "w-20 sm:w-24 md:w-32"
        }`}
        style={{
          fontSize: "clamp(8rem, 16vw, 14rem)",
          color: "transparent",
          WebkitTextStrokeWidth: "3px",
          WebkitTextStrokeColor: "rgba(180,180,180,0.55)",
          transform: "translateY(0.5rem)",
        }}
      >
        {rank}
      </div>
      <div className="-ml-4 sm:-ml-5 md:-ml-6">
        <Card item={item} variant="poster" />
      </div>
    </div>
  );
}
