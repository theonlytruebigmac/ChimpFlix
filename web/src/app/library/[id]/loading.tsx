import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";

export default function LibraryLoading() {
  return (
    <main className="relative">
      <HeroSkeleton />
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <RailSkeleton title="Recently Added" />
        <RailSkeleton />
      </div>
    </main>
  );
}
