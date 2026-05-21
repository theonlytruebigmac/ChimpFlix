import { RailSkeleton } from "@/components/Skeleton";
import { brandName } from "@/lib/env";

export default function NewPopularLoading() {
  return (
    <main className="relative min-h-screen bg-background">
      <div className="pb-24 pt-24 sm:pt-28">
        <h1 className="mb-8 px-4 sm:px-8 md:px-12 text-4xl font-bold tracking-tight">
          New &amp; Popular
        </h1>
        <div className="space-y-1">
          <RailSkeleton title={`New on ${brandName()}`} />
          <RailSkeleton title="Top 10 Movies Today" />
          <RailSkeleton title="Top 10 TV Shows Today" />
        </div>
      </div>
    </main>
  );
}
