import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";

export default function MoviesLoading() {
  return (
    <main className="relative">
      <TopNav />
      <HeroSkeleton />
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <RailSkeleton title="Recently Added" />
        <RailSkeleton />
      </div>
    </main>
  );
}
