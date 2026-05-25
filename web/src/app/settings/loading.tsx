/// Settings-scoped loading skeleton. Mirrors the typical settings
/// panel silhouette — page header strip + ~3 grouped sections —
/// so the layout doesn't jump when the real client component
/// streams in.
export default function SettingsLoading() {
  return (
    <main className="px-4 py-8 text-white">
      <div className="mx-auto max-w-3xl animate-pulse space-y-6">
        <div>
          <div className="h-7 w-40 rounded bg-white/10" />
          <div className="mt-2 h-4 w-72 rounded bg-white/5" />
        </div>
        {[0, 1, 2].map((i) => (
          <section
            key={i}
            className="space-y-3 rounded-lg border border-white/10 bg-white/2 p-6"
          >
            <div className="h-4 w-32 rounded bg-white/10" />
            <div className="h-9 rounded bg-white/5" />
            <div className="h-9 rounded bg-white/5" />
          </section>
        ))}
      </div>
    </main>
  );
}
