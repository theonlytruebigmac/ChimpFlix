/// My-list grid skeleton. Reserves space for the page header + a
/// poster grid (~12 tiles) so the layout doesn't reflow when the
/// real data resolves.
export default function MyListLoading() {
  return (
    <main className="px-4 pt-20 pb-12 text-white">
      <div className="mx-auto max-w-7xl animate-pulse space-y-6">
        <div>
          <div className="h-7 w-32 rounded bg-white/10" />
          <div className="mt-2 h-4 w-72 rounded bg-white/5" />
        </div>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
          {Array.from({ length: 12 }).map((_, i) => (
            <div key={i} className="aspect-[2/3] rounded bg-white/10" />
          ))}
        </div>
      </div>
    </main>
  );
}
