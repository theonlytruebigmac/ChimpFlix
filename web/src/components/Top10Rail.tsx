"use client";

import type { MediaItem } from "@/lib/chimpflix-types";
import { Card } from "./Card";

/// Netflix-style "Top 10" rail: each entry is a card with a giant
/// outlined rank glyph sitting behind/beside it. The number uses CSS
/// text stroke (-webkit-text-stroke) so it reads as outline-only
/// without needing per-character SVGs. Falls back to a heavy filled
/// number in browsers without text-stroke support, which is still
/// recognizably the Netflix design.
///
/// The list is short (max 10), curated to titles the local server
/// actually holds, ranked by global TMDB trending. Empty rail renders
/// nothing so the homepage doesn't show a hollow shelf.
export function Top10Rail({
  title,
  items,
}: {
  title: string;
  items: Array<{ rank: number; item: MediaItem }>;
}) {
  if (items.length === 0) return null;
  return (
    <section
      className="zf-rise-in px-4 sm:px-8 md:px-12 pb-1 pt-1"
      style={{ contentVisibility: "auto", containIntrinsicSize: "260px" }}
    >
      <h2 className="mb-3 flex items-baseline gap-2 text-[1.4rem] font-semibold tracking-tight">
        <span>{title}</span>
        <span className="text-xs font-medium uppercase tracking-[0.18em] text-white/45">
          Top 10
        </span>
      </h2>
      <ul className="-mx-1 flex gap-1.5 overflow-x-auto px-1 pb-12 pt-2 scrollbar-none [&::-webkit-scrollbar]:hidden">
        {items.map(({ rank, item }) => (
          <li
            key={item.ratingKey}
            className="flex-none [&:first-child_.card-scaler]:origin-left [&:last-child_.card-scaler]:origin-right"
          >
            <Top10Entry rank={rank} item={item} />
          </li>
        ))}
      </ul>
    </section>
  );
}

function Top10Entry({ rank, item }: { rank: number; item: MediaItem }) {
  // 10 is two-glyph, so it needs more horizontal real estate behind the
  // card than 1-9. We push the card right by a constant for ranks 1-9
  // and a wider one for 10. The glyph itself is positioned absolutely
  // behind the card; the card stays at the natural Card width (w-72).
  const isTwoDigit = rank >= 10;
  return (
    <div className="relative flex items-stretch">
      <div
        aria-hidden
        className={`pointer-events-none relative shrink-0 select-none font-black leading-none ${
          isTwoDigit ? "w-44" : "w-28"
        }`}
        style={{
          fontSize: "14rem",
          color: "transparent",
          WebkitTextStrokeWidth: "3px",
          WebkitTextStrokeColor: "rgba(180,180,180,0.55)",
          // Tuck the number against the card and bleed it slightly off
          // the rail's baseline so it feels integrated rather than
          // floating above.
          transform: "translateY(0.5rem)",
        }}
      >
        {rank}
      </div>
      <div className="-ml-6">
        <Card item={item} />
      </div>
    </div>
  );
}
