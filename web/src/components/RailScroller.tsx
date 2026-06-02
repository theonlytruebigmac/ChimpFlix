"use client";

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

/// Horizontal rail viewport with Netflix-style "page" arrows.
///
/// Wraps the tile `<ul>` (the same scroll container the server rails used
/// to render inline) and overlays a left/right chevron on desktop. Each
/// press advances by exactly one page — as many WHOLE tiles as currently
/// fit in the viewport — so the rail jumps a fixed, predictable amount
/// instead of a fractional drag. The page size is measured from the live
/// DOM (distance between adjacent tile starts) so it tracks the responsive
/// tile sizing without hard-coding a per-rail count.
///
/// Arrows are desktop-only — touch users swipe (and the scroll container
/// is `touch-pan-x` locked so a vertical swipe scrolls the page, not the
/// rail). Each arrow only renders for a direction that can actually scroll.
export function RailScroller({
  className,
  children,
}: {
  className?: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLUListElement>(null);
  const [canLeft, setCanLeft] = useState(false);
  const [canRight, setCanRight] = useState(false);

  const update = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    // 4px slack so sub-pixel scroll maxima don't strand a dead arrow.
    setCanLeft(el.scrollLeft > 4);
    setCanRight(el.scrollLeft + el.clientWidth < el.scrollWidth - 4);
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    update();
    el.addEventListener("scroll", update, { passive: true });
    // The parent rail uses content-visibility:auto, so the container has
    // zero size until scrolled into view — the ResizeObserver re-runs the
    // check the moment it materialises and on every responsive reflow.
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => {
      el.removeEventListener("scroll", update);
      ro.disconnect();
    };
  }, [update]);

  const page = useCallback((dir: 1 | -1) => {
    const el = ref.current;
    if (!el) return;
    const items = el.children;
    let stride = el.clientWidth;
    if (items.length >= 2) {
      // Tile width + inter-tile gap, straight from layout.
      stride =
        (items[1] as HTMLElement).offsetLeft -
        (items[0] as HTMLElement).offsetLeft;
    } else if (items.length === 1) {
      stride = (items[0] as HTMLElement).offsetWidth;
    }
    if (stride <= 0) stride = el.clientWidth;
    const perPage = Math.max(1, Math.floor(el.clientWidth / stride));
    el.scrollBy({ left: dir * perPage * stride, behavior: "smooth" });
  }, []);

  return (
    <div className="group/rail relative">
      <ul ref={ref} className={className}>
        {children}
      </ul>
      {canLeft && <RailArrow dir="left" onClick={() => page(-1)} />}
      {canRight && <RailArrow dir="right" onClick={() => page(1)} />}
    </div>
  );
}

function RailArrow({
  dir,
  onClick,
}: {
  dir: "left" | "right";
  onClick: () => void;
}) {
  // Full class strings per side — Tailwind only emits classes it can find
  // as complete literals, so these can't be interpolated.
  const base =
    "absolute top-2 bottom-12 z-10 hidden w-12 items-center justify-center text-white/90 opacity-0 transition-opacity duration-200 hover:text-white focus-visible:opacity-100 focus-visible:outline-none group-hover/rail:opacity-100 md:flex";
  const sided =
    dir === "left"
      ? `${base} left-0 bg-linear-to-r from-black/70 to-transparent`
      : `${base} right-0 bg-linear-to-l from-black/70 to-transparent`;
  return (
    <button
      type="button"
      aria-label={dir === "left" ? "Scroll left" : "Scroll right"}
      onClick={onClick}
      className={sided}
    >
      <span className="flex h-9 w-9 items-center justify-center rounded-full bg-black/50 backdrop-blur-sm">
        <svg
          width="22"
          height="22"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          {dir === "left" ? (
            <polyline points="15 18 9 12 15 6" />
          ) : (
            <polyline points="9 18 15 12 9 6" />
          )}
        </svg>
      </span>
    </button>
  );
}
