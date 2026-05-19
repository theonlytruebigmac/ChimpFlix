import { CardSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";

export default function GenreLoading() {
  return (
    <main className="relative min-h-screen bg-background">
      <TopNav />
      <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
        <div className="mb-10 h-10 w-48 rounded bg-white/10" />
        <ul className="flex flex-wrap gap-3">
          {Array.from({ length: 12 }).map((_, i) => (
            <li key={i} className="flex-none">
              <CardSkeleton />
            </li>
          ))}
        </ul>
      </div>
    </main>
  );
}
