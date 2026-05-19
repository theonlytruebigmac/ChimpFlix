import { TopNav } from "@/components/TopNav";

export default function SearchLoading() {
  return (
    <main className="relative min-h-screen bg-background">
      <TopNav />
      <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
        <div className="mb-3 h-10 w-32 rounded bg-white/10" />
        <p className="text-white/40">Searching…</p>
      </div>
    </main>
  );
}
