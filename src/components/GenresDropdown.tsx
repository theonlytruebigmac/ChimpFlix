"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";

/**
 * Page-scoped genres picker shown next to the Shows / Movies / Genre page
 * heading. Clicking a genre navigates to /genre/<name>?type=<type>; the
 * "All" option goes back to the type's index (/shows or /movies). The
 * `current` prop is just the active label — purely cosmetic, so the
 * trigger reads "Action ▾" instead of always "Genres ▾" when filtered.
 */
export function GenresDropdown({
  genres,
  type,
  current,
}: {
  genres: readonly string[];
  type: "movie" | "show";
  current?: string | null;
}) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!containerRef.current?.contains(e.target as Node)) setOpen(false);
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open]);

  const allLabel = type === "show" ? "All Shows" : "All Movies";
  const allHref = type === "show" ? "/shows" : "/movies";

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        className="flex items-center gap-2 rounded-sm border border-white/40 bg-black/30 px-3 py-1.5 text-sm font-medium text-white/90 transition-colors hover:border-white hover:text-white"
      >
        <span>{current || "Genres"}</span>
        <svg
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="3"
          aria-hidden
          className={`transition-transform duration-150 ${open ? "rotate-180" : ""}`}
        >
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>

      {open && (
        <div
          role="menu"
          className="absolute left-0 top-full z-50 mt-2 max-h-[70vh] w-56 overflow-y-auto overflow-hidden rounded-sm border border-white/15 bg-(--color-surface) shadow-2xl"
        >
          <Link
            href={allHref}
            role="menuitem"
            onClick={() => setOpen(false)}
            className={`block px-4 py-2 text-sm transition-colors hover:bg-white/10 hover:text-white ${
              current ? "text-white/85" : "font-semibold text-white"
            }`}
          >
            {allLabel}
          </Link>
          <div className="my-1 border-t border-white/10" />
          {genres.map((g) => {
            const active = current === g;
            return (
              <Link
                key={g}
                href={`/genre/${encodeURIComponent(g)}?type=${type}`}
                role="menuitem"
                onClick={() => setOpen(false)}
                className={`block px-4 py-2 text-sm transition-colors hover:bg-white/10 hover:text-white ${
                  active ? "font-semibold text-white" : "text-white/85"
                }`}
              >
                {g}
              </Link>
            );
          })}
        </div>
      )}
    </div>
  );
}
