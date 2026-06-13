import Link from "next/link";

/// Page-header strip used at the top of /movies and /shows. Combines
/// the section title (Movies / Shows) with a horizontal chip bar that
/// jumps into the shared `/genre/{name}` filtered view. Netflix puts
/// the equivalent title + "All Genres ▾" dropdown in this same spot;
/// we list the chips inline because our genre count is small enough
/// that a dropdown would be heavier than the value.
///
/// Lives just under TopNav and above the hero, with enough top padding
/// to clear the fixed nav. The hero sits underneath the chips on
/// scroll, so the chips don't compete with the hero artwork.
export function GenreFilterBar({
  title,
  genres,
  allHref,
}: {
  title: string;
  genres: string[];
  allHref: string;
}) {
  return (
    <div className="relative z-30 pt-24">
      <div className="flex items-center gap-3 px-4 pb-3 sm:px-8 md:px-12">
        <h1 className="text-3xl font-bold tracking-tight">{title}</h1>
        <div className="ml-2 flex min-w-0 items-center gap-1.5 overflow-x-auto overscroll-x-contain touch-pan-x scrollbar-none [&::-webkit-scrollbar]:hidden">
          <Chip href={allHref} label="All" active />
          {genres.map((g) => (
            <Chip
              key={g}
              href={`/genre/${encodeURIComponent(g)}`}
              label={g}
              active={false}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function Chip({
  href,
  label,
  active,
}: {
  href: string;
  label: string;
  active: boolean;
}) {
  return (
    <Link
      href={href}
      className={`shrink-0 rounded-sm border px-2.5 py-1 text-xs font-medium transition-colors ${
        active
          ? "border-white bg-white/10 text-white"
          : "border-white/30 text-white/80 hover:border-white hover:text-white"
      }`}
    >
      {label}
    </Link>
  );
}
