import { Suspense } from "react";
import { ModalRoot } from "@/components/ModalRoot";
import { Rail } from "@/components/Rail";
import { RailSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";
import { brandName } from "@/lib/env";
import {
  items as itemsApi,
  libraries as librariesApi,
  type Library,
} from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";

const RAIL_SIZE = 20;

export default async function NewPopularPage() {
  await requireUser("/new-popular");
  const { libraries } = await librariesApi.list();

  return (
    <main className="relative min-h-screen bg-background">
      <TopNav />
      <div className="pb-24 pt-28">
        <h1 className="mb-8 px-12 text-4xl font-bold tracking-tight">
          New &amp; Popular
        </h1>
        <div className="space-y-1">
          <Suspense fallback={<RailSkeleton title={`New on ${brandName()}`} />}>
            <NewOnBrandRail />
          </Suspense>
          <Suspense fallback={<RailSkeleton title="Top Rated Movies" />}>
            <TopRatedRail kind="movie" title="Top Rated Movies" />
          </Suspense>
          <Suspense fallback={<RailSkeleton title="Top Rated Shows" />}>
            <TopRatedRail kind="show" title="Top Rated Shows" />
          </Suspense>
          {libraries.map((lib) => (
            <Suspense
              key={`lib-${lib.id}`}
              fallback={<RailSkeleton title={`New in ${lib.name}`} />}
            >
              <LibraryRail lib={lib} />
            </Suspense>
          ))}
        </div>
      </div>
      <ModalRoot />
    </main>
  );
}

async function NewOnBrandRail() {
  const res = await itemsApi.list({ page_size: RAIL_SIZE });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={`New on ${brandName()}`} items={items} />;
}

async function TopRatedRail({
  kind,
  title,
}: {
  kind: "movie" | "show";
  title: string;
}) {
  const res = await itemsApi.list({
    kind,
    sort: "rating_desc",
    page_size: RAIL_SIZE,
  });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={title} items={items} />;
}

async function LibraryRail({ lib }: { lib: Library }) {
  const res = await itemsApi.list({
    library_id: lib.id,
    page_size: RAIL_SIZE,
  });
  const items = res.items.map(adaptItem);
  if (items.length === 0) return null;
  return <Rail title={`New in ${lib.name}`} items={items} />;
}
