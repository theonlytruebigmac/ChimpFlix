import { HeroSkeleton, RailSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";

// Shown by Next during cold first-byte for the home page (before our own
// per-rail Suspense boundaries take over). Renders the same TopNav as
// the loaded page so there's no visual jump when content streams in.
export default function HomeLoading() {
  return (
    <main className="relative">
      <TopNav />
      <HeroSkeleton />
      <div className="relative z-20 space-y-1 pb-24 pt-4">
        <RailSkeleton title="Continue Watching" />
        <RailSkeleton title="Recently Added" />
        <RailSkeleton />
      </div>
    </main>
  );
}
