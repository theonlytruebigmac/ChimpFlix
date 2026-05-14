import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";

export default function ShowsLoading() {
  return (
    <main className="relative">
      <TopNav />
      <div className="relative z-20 flex items-baseline gap-4 px-12 pt-24 pb-2">
        <h1 className="text-3xl font-bold tracking-tight">Shows</h1>
        <div className="h-7 w-24 rounded-sm border border-white/15 bg-white/5" />
      </div>
      <HeroSkeleton />
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <RailSkeleton title="Recently Added" />
        <RailSkeleton />
      </div>
    </main>
  );
}
