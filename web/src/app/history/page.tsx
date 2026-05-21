import { Suspense } from "react";
import { Card } from "@/components/Card";
import { ModalRoot } from "@/components/ModalRoot";
import { CardSkeleton } from "@/components/Skeleton";
import { playState as playStateApi } from "@/lib/chimpflix-api";
import { adaptItem } from "@/lib/chimpflix-adapt";
import { requireUser } from "@/lib/chimpflix-server";

export default async function HistoryPage() {
  await requireUser("/history");

  return (
    <main className="relative min-h-screen bg-background">
      <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
        <h1 className="mb-6 text-4xl font-bold tracking-tight">
          Watch History
        </h1>
        <Suspense fallback={<Grid skeleton />}>
          <HistoryGrid />
        </Suspense>
      </div>
      <ModalRoot />
    </main>
  );
}

async function HistoryGrid() {
  const { items } = await playStateApi.history(120);
  if (items.length === 0) {
    return (
      <p className="text-white/60">
        Nothing watched yet. Play something and it&apos;ll show up here.
      </p>
    );
  }
  const adapted = items.map(adaptItem);
  return (
    <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
      {adapted.map((it) => (
        <li key={it.ratingKey}>
          <Card item={it} />
        </li>
      ))}
    </ul>
  );
}

function Grid({ skeleton }: { skeleton?: boolean }) {
  if (!skeleton) return null;
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
