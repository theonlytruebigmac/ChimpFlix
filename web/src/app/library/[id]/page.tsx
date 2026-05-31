import { notFound } from "next/navigation";
import Link from "next/link";
import { Suspense } from "react";
import { Hero } from "@/components/Hero";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
import { Top10Rail } from "@/components/Top10Rail";
import { pickHeroIndex } from "@/lib/hero";
import {
  items as itemsApi,
  libraries as librariesApi,
  type ItemKind,
  type Library,
} from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";

const RAIL_PAGE_SIZE = 20;

const MOVIE_GENRES = [
  "Action",
  "Comedy",
  "Drama",
  "Thriller",
  "Science Fiction",
  "Horror",
  "Adventure",
];
const SHOW_GENRES = [
  "Drama",
  "Comedy",
  "Animation",
  "Crime",
  "Mystery",
  "Science Fiction",
  "Family",
];

function itemKindFor(lib: Library): ItemKind {
  // Anime libraries are treated as series (show-kind items).
  return lib.kind === "movies" ? "movie" : "show";
}

function genresFor(lib: Library): readonly string[] {
  return lib.kind === "movies" ? MOVIE_GENRES : SHOW_GENRES;
}

export default async function LibraryPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id: idStr } = await params;
  const id = Number(idStr);
  if (!Number.isFinite(id) || id <= 0) notFound();

  await requireUser(`/library/${id}`);

  // Use the access-filtered list endpoint so a user requesting a library
  // they can't see naturally falls through to 404, matching the API.
  const { libraries } = await librariesApi.list();
  const lib = libraries.find((l) => l.id === id);
  if (!lib) notFound();

  const kind = itemKindFor(lib);
  const genres = genresFor(lib);

  return (
    <main className="relative">
      <Suspense fallback={<HeroSkeleton />}>
        <LibraryHero lib={lib} kind={kind} />
      </Suspense>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <Suspense fallback={null}>
          <LibraryTop10Rail lib={lib} />
        </Suspense>
        <Suspense fallback={<RailSkeleton title="Recently Added" />}>
          <RecentlyAddedRail lib={lib} kind={kind} />
        </Suspense>
        {genres.map((g) => (
          <Suspense key={`genre-${g}`} fallback={null}>
            <GenreRail lib={lib} kind={kind} genre={g} />
          </Suspense>
        ))}
        {/*
          "Browse all" call-out — full inventory grid for finding a
          specific title or fixing unmatched files. Sits below the
          rails so it doesn't pre-empt the Netflix-style discovery
          flow at the top, but stays visible (not buried in a menu).
        */}
        <div className="px-4 pt-10 pb-2 sm:px-8 md:px-12">
          <Link
            href={`/library/${lib.id}/browse`}
            className="inline-flex items-center gap-2 rounded-md border border-white/15 bg-white/2 px-4 py-2.5 text-sm font-medium text-white/85 transition-colors hover:border-white/35 hover:bg-white/5 hover:text-white"
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden
            >
              <rect x="3" y="3" width="7" height="7" />
              <rect x="14" y="3" width="7" height="7" />
              <rect x="3" y="14" width="7" height="7" />
              <rect x="14" y="14" width="7" height="7" />
            </svg>
            Browse all titles
            <span className="ml-1 text-white/45">→</span>
          </Link>
        </div>
      </div>
      <ModalRoot />
    </main>
  );
}

async function LibraryHero({ lib, kind }: { lib: Library; kind: ItemKind }) {
  const res = await itemsApi.list({
    library_id: lib.id,
    kind,
    page_size: 12,
  });
  // Same rationale as HomeHero: accept items with art or thumb (Hero
  // falls back to thumb internally), and render a nav-height spacer if
  // the library is truly empty so the rails section clears the fixed
  // TopNav.
  const pool = res.items
    .map(adaptItem)
    .filter((it) => Boolean(it.art) || Boolean(it.thumb))
    .sort((a, b) => (a.art ? -1 : 0) - (b.art ? -1 : 0))
    .slice(0, 5);
  if (pool.length === 0) {
    return <div className="h-20 md:h-24" aria-hidden />;
  }
  return <Hero item={pool[pickHeroIndex(pool, `library-${lib.id}`)]} />;
}

/// Per-library, type-aware Top 10. The source is decided server-side
/// from the library's kind (Movies/Shows → TMDB top-rated, Anime →
/// MyAnimeList ranking), blended with the library's local top-watched.
/// Renders nothing when the source hasn't been refreshed / configured
/// (the endpoint returns an empty list, not an error), so a fresh
/// install or a key-less MAL setup just shows no rail.
async function LibraryTop10Rail({ lib }: { lib: Library }) {
  let entries: Array<{ rank: number; item: ReturnType<typeof adaptItem> }>;
  try {
    const res = await itemsApi.libraryTop(lib.id, 10);
    // Dedupe by local item id (always present; tmdb_id may be null for
    // anime) and re-rank to a clean 1..N for the Netflix-style numerals.
    const seen = new Set<number>();
    const unique = res.items.filter((it) => {
      if (seen.has(it.id)) return false;
      seen.add(it.id);
      return true;
    });
    entries = unique.map(({ rank: _rank, ...item }, idx) => ({
      rank: idx + 1,
      item: adaptItem(item),
    }));
  } catch {
    return null;
  }
  if (entries.length === 0) return null;
  return (
    <Top10Rail
      title="Top 10"
      items={entries}
      href={`/library/${lib.id}/browse?sort=rating_desc`}
    />
  );
}

async function RecentlyAddedRail({
  lib,
  kind,
}: {
  lib: Library;
  kind: ItemKind;
}) {
  const res = await itemsApi.list({
    library_id: lib.id,
    kind,
    page_size: RAIL_PAGE_SIZE,
  });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title="Recently Added" items={items} />;
}

async function GenreRail({
  lib,
  kind,
  genre,
}: {
  lib: Library;
  kind: ItemKind;
  genre: string;
}) {
  const res = await itemsApi.list({
    library_id: lib.id,
    kind,
    genre,
    page_size: 16,
  });
  const items = res.items.map(adaptItem);
  if (items.length < 4) return null;
  return (
    <Rail
      title={genre}
      items={items}
      href={`/genre/${encodeURIComponent(genre)}`}
    />
  );
}
