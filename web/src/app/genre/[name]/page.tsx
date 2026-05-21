import { Suspense } from "react";
import { Card } from "@/components/Card";
import { ModalRoot } from "@/components/ModalRoot";
import { CardSkeleton } from "@/components/Skeleton";
import { items as itemsApi } from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";

const RESULTS_PAGE_SIZE = 60;

export default async function GenrePage({
  params,
}: {
  params: Promise<{ name: string }>;
}) {
  const { name } = await params;
  const genre = decodeURIComponent(name);
  await requireUser(`/genre/${name}`);

  return (
    <main className="relative min-h-screen bg-background">
      <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
        <h1 className="mb-6 text-4xl font-bold tracking-tight">{genre}</h1>
        <Suspense fallback={<ResultsSkeleton />}>
          <GenreGrid genre={genre} />
        </Suspense>
      </div>
      <ModalRoot />
    </main>
  );
}

async function GenreGrid({ genre }: { genre: string }) {
  const res = await itemsApi.list({ genre, page_size: RESULTS_PAGE_SIZE });
  if (res.items.length === 0) {
    return <p className="text-white/70">Nothing in this genre yet.</p>;
  }
  const items = res.items.map(adaptItem);
  return (
    <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
      {items.map((it) => (
        <li key={it.ratingKey}>
          <Card item={it} />
        </li>
      ))}
    </ul>
  );
}

function ResultsSkeleton() {
  return (
    <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
      {Array.from({ length: 12 }).map((_, i) => (
        <li key={i}>
          <CardSkeleton />
        </li>
      ))}
    </ul>
  );
}
