/// Watch-history skeleton. Reserves space for the page header + a
/// vertical list of N rows (each row = poster strip + title + meta)
/// so the layout doesn't pop when the real data lands.
export default function HistoryLoading() {
  return (
    <main className="px-4 pt-20 pb-12 text-white">
      <div className="mx-auto max-w-5xl animate-pulse space-y-6">
        <div>
          <div className="h-7 w-40 rounded bg-white/10" />
          <div className="mt-2 h-4 w-56 rounded bg-white/5" />
        </div>
        <ul className="space-y-3">
          {Array.from({ length: 8 }).map((_, i) => (
            <li
              key={i}
              className="flex items-center gap-4 rounded-lg border border-white/5 bg-white/2 p-3"
            >
              <div className="h-16 w-28 shrink-0 rounded bg-white/10" />
              <div className="flex-1 space-y-2">
                <div className="h-4 w-1/2 rounded bg-white/10" />
                <div className="h-3 w-1/3 rounded bg-white/5" />
              </div>
            </li>
          ))}
        </ul>
      </div>
    </main>
  );
}
