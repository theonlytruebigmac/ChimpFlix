import { Suspense } from "react";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { RailSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";
import { brandName } from "@/lib/env";
import {
  filterHiddenItems,
  readHiddenLibraries,
} from "@/lib/library-prefs";
import {
  recentlyAdded,
  sectionRecentlyAdded,
  sectionTopRated,
  sectionTopWatched,
  sections,
  type ServerAuth,
} from "@/lib/plex-data";
import { requireServerAuth } from "@/lib/session";

export default async function NewPopularPage() {
  const t0 = Date.now();
  const auth = await requireServerAuth();
  const t1 = Date.now();
  const hidden = await readHiddenLibraries();
  console.log(
    `[perf] /new-popular auth=${t1 - t0}ms readHidden=${Date.now() - t1}ms`,
  );

  return (
    <main className="relative min-h-screen bg-black">
      <TopNav />
      <div className="pb-24 pt-28">
        <h1 className="mb-8 px-12 text-4xl font-bold tracking-tight">
          New &amp; Popular
        </h1>
        <div className="space-y-1">
          <Suspense fallback={<RailSkeleton title={`New on ${brandName()}`} />}>
            <NewOnBrandRail auth={auth} hidden={hidden} />
          </Suspense>
          <Suspense fallback={<RailSkeleton title="Top 10 Movies Today" />}>
            <Top10Rail
              auth={auth}
              hidden={hidden}
              type="movie"
              title="Top 10 Movies Today"
            />
          </Suspense>
          <Suspense fallback={<RailSkeleton title="Top 10 TV Shows Today" />}>
            <Top10Rail
              auth={auth}
              hidden={hidden}
              type="show"
              title="Top 10 TV Shows Today"
            />
          </Suspense>
          <Suspense fallback={<RailSkeleton title="Top Rated Movies" />}>
            <TopRatedRail
              auth={auth}
              hidden={hidden}
              type="movie"
              title="Top Rated Movies"
            />
          </Suspense>
          <Suspense fallback={<RailSkeleton title="Top Rated TV Shows" />}>
            <TopRatedRail
              auth={auth}
              hidden={hidden}
              type="show"
              title="Top Rated TV Shows"
            />
          </Suspense>
          <Suspense fallback={null}>
            <PerLibraryNew auth={auth} hidden={hidden} />
          </Suspense>
        </div>
      </div>
      <ModalRoot />
    </main>
  );
}

async function NewOnBrandRail({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const t0 = Date.now();
  const items = filterHiddenItems(await recentlyAdded(auth), hidden);
  console.log(
    `[perf] /new-popular NewOnBrandRail recentlyAdded=${Date.now() - t0}ms`,
  );
  if (items.length === 0) return null;
  return <Rail title={`New on ${brandName()}`} items={items} />;
}

async function Top10Rail({
  auth,
  hidden,
  type,
  title,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
  type: "movie" | "show";
  title: string;
}) {
  const t0 = Date.now();
  const libs = (await sections(auth)).filter(
    (s) => s.type === type && !hidden.has(s.key),
  );
  const perLib = await Promise.all(
    libs.map((lib) => sectionTopWatched(auth, lib.key, 10)),
  );
  console.log(
    `[perf] /new-popular Top10Rail(${type}) sections+topWatched=${Date.now() - t0}ms`,
  );
  const items = perLib.find((arr) => arr.length > 0) ?? [];
  if (items.length < 4) return null;
  return <Rail title={title} items={items} />;
}

async function TopRatedRail({
  auth,
  hidden,
  type,
  title,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
  type: "movie" | "show";
  title: string;
}) {
  const t0 = Date.now();
  const libs = (await sections(auth)).filter(
    (s) => s.type === type && !hidden.has(s.key),
  );
  const perLib = await Promise.all(
    libs.map((lib) => sectionTopRated(auth, lib.key, 24)),
  );
  console.log(
    `[perf] /new-popular TopRatedRail(${type}) sections+topRated=${Date.now() - t0}ms`,
  );
  const items = (perLib.find((arr) => arr.length > 0) ?? [])
    .filter((it) => typeof it.rating === "number" && it.rating > 0)
    .slice(0, 10);
  if (items.length < 4) return null;
  return <Rail title={title} items={items} />;
}

async function PerLibraryNew({
  auth,
  hidden,
}: {
  auth: ServerAuth;
  hidden: Set<string>;
}) {
  const t0 = Date.now();
  const libs = (await sections(auth)).filter((s) => !hidden.has(s.key));
  const data = await Promise.all(
    libs.map(async (lib) => ({
      lib,
      items: await sectionRecentlyAdded(auth, lib.key),
    })),
  );
  console.log(
    `[perf] /new-popular PerLibraryNew(${libs.length} libs)=${Date.now() - t0}ms`,
  );
  return (
    <>
      {data.map(({ lib, items }) =>
        items.length >= 4 ? (
          <Rail
            key={`new-in-${lib.key}`}
            title={`New in ${lib.title}`}
            items={items}
          />
        ) : null,
      )}
    </>
  );
}
