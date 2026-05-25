/// Person-page skeleton. Reserves space for the headshot + name +
/// known-for grid so the layout doesn't reflow when the real data
/// streams in.
export default function PersonLoading() {
  return (
    <main className="px-4 pt-20 pb-12 text-white">
      <div className="mx-auto max-w-7xl animate-pulse space-y-8">
        <div className="flex items-end gap-6">
          <div className="h-40 w-40 shrink-0 rounded-full bg-white/10" />
          <div className="flex-1 space-y-3 pb-2">
            <div className="h-8 w-64 rounded bg-white/10" />
            <div className="h-4 w-40 rounded bg-white/5" />
            <div className="h-3 w-80 rounded bg-white/5" />
          </div>
        </div>
        <div>
          <div className="mb-4 h-5 w-32 rounded bg-white/10" />
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
            {Array.from({ length: 12 }).map((_, i) => (
              <div key={i} className="aspect-[2/3] rounded bg-white/10" />
            ))}
          </div>
        </div>
      </div>
    </main>
  );
}
