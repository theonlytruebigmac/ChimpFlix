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
  prefs as prefsApi,
  type Library,
} from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";
import type { MediaItem } from "@/lib/chimpflix-types";

const RAIL_PAGE_SIZE = 20;
const GENRES = [
  "Drama",
  "Comedy",
  "Animation",
  "Crime",
  "Mystery",
  "Science Fiction",
  "Family",
];

export default async function ShowsPage() {
  await requireUser("/shows");
  const [{ libraries }, { library_ids: hiddenIds }] = await Promise.all([
    librariesApi.list(),
    prefsApi.hiddenLibraries(),
  ]);
  const hidden = new Set(hiddenIds);
  const showLibs = libraries.filter(
    (l) => l.kind === "shows" && !hidden.has(l.id),
  );

  return (
    <main className="relative">
      <TopNav />
      <Suspense fallback={<HeroSkeleton />}>
        <ShowsHero />
      </Suspense>
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <Suspense fallback={<RailSkeleton title="Recently Added" />}>
          <RecentlyAddedRail />
        </Suspense>
        {showLibs.map((lib) => (
          <Suspense
            key={`lib-${lib.id}`}
            fallback={<RailSkeleton title={`New in ${lib.name}`} />}
          >
            <LibraryRail lib={lib} />
          </Suspense>
        ))}
        {GENRES.map((g) => (
          <Suspense key={`genre-${g}`} fallback={null}>
            <GenreRail genre={g} />
          </Suspense>
        ))}
      </div>
      <ModalRoot />
    </main>
  );
}

async function ShowsHero() {
  const res = await itemsApi.list({ kind: "show", page_size: 12 });
  const pool = res.items
    .map(adaptItem)
    .filter((it): it is MediaItem & { art: string } => Boolean(it.art))
    .slice(0, 5);
  if (pool.length === 0) return null;
  return <Hero item={pool[pickHeroIndex(pool, "shows")]} />;
}

async function RecentlyAddedRail() {
  const res = await itemsApi.list({ kind: "show", page_size: RAIL_PAGE_SIZE });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title="Recently Added" items={items} />;
}

async function LibraryRail({ lib }: { lib: Library }) {
  const res = await itemsApi.list({
    library_id: lib.id,
    kind: "show",
    page_size: RAIL_PAGE_SIZE,
  });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={`New in ${lib.name}`} items={items} />;
}

async function GenreRail({ genre }: { genre: string }) {
  const res = await itemsApi.list({
    kind: "show",
    genre,
    page_size: 16,
  });
  const items = res.items.map(adaptItem);
  if (items.length < 4) return null;
  return (
    <Rail title={genre} items={items} href={`/genre/${encodeURIComponent(genre)}`} />
  );
}
