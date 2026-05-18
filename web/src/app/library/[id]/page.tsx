import { notFound } from "next/navigation";
import { Suspense } from "react";
import { Hero } from "@/components/Hero";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";
import { pickHeroIndex } from "@/lib/hero";
import {
  items as itemsApi,
  libraries as librariesApi,
  type ItemKind,
  type Library,
} from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";
import type { MediaItem } from "@/lib/chimpflix-types";

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
      <TopNav />
      <Suspense fallback={<HeroSkeleton />}>
        <LibraryHero lib={lib} kind={kind} />
      </Suspense>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <Suspense fallback={<RailSkeleton title="Recently Added" />}>
          <RecentlyAddedRail lib={lib} kind={kind} />
        </Suspense>
        {genres.map((g) => (
          <Suspense key={`genre-${g}`} fallback={null}>
            <GenreRail lib={lib} kind={kind} genre={g} />
          </Suspense>
        ))}
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
  const pool = res.items
    .map(adaptItem)
    .filter((it): it is MediaItem & { art: string } => Boolean(it.art))
    .slice(0, 5);
  if (pool.length === 0) return null;
  return <Hero item={pool[pickHeroIndex(pool, `library-${lib.id}`)]} />;
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
