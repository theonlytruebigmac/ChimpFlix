import Link from "next/link";

/// Netflix-style "More to explore" chip row above search results.
/// Curated list of jumping-off points (popular genres + New & Popular
/// destinations) so users have something to click before they finish
/// typing. Matches Netflix's chip styling — pill-shaped, dark fill,
/// thin border, horizontal scroll on overflow.
const CHIPS: Array<{ label: string; href: string }> = [
  { label: "Trending Now", href: "/new-popular" },
  { label: "New Releases", href: "/new-popular" },
  { label: "Action", href: "/genre/Action" },
  { label: "Comedy", href: "/genre/Comedy" },
  { label: "Drama", href: "/genre/Drama" },
  { label: "Thriller", href: "/genre/Thriller" },
  { label: "Science Fiction", href: "/genre/Science%20Fiction" },
  { label: "Horror", href: "/genre/Horror" },
  { label: "Animation", href: "/genre/Animation" },
  { label: "Family", href: "/genre/Family" },
  { label: "Documentary", href: "/genre/Documentary" },
  { label: "Mystery", href: "/genre/Mystery" },
];

export function MoreToExploreChips() {
  return (
    <div className="mb-6 flex items-center gap-3 overflow-x-auto overscroll-x-contain touch-pan-x pb-1 scrollbar-none [&::-webkit-scrollbar]:hidden">
      <span className="shrink-0 text-[0.78rem] uppercase tracking-wider text-white/45">
        More to explore
      </span>
      {CHIPS.map((chip) => (
        <Link
          key={chip.label}
          href={chip.href}
          className="shrink-0 rounded-full border border-white/15 bg-white/5 px-3 py-1 text-xs font-medium text-white/85 transition-colors hover:border-white hover:bg-white/10 hover:text-white"
        >
          {chip.label}
        </Link>
      ))}
    </div>
  );
}
