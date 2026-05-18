import type { LibraryHealthResponse } from "@/lib/chimpflix-api";

export function AdminLibraryHealthClient({
  report,
}: {
  report: LibraryHealthResponse;
}) {
  const counters: Array<{ label: string; value: number; tone: "ok" | "warn" }> = [
    {
      label: "Items without files",
      value: report.items_without_files,
      tone: report.items_without_files > 0 ? "warn" : "ok",
    },
    {
      label: "Items missing every metadata id",
      value: report.items_without_metadata,
      tone: report.items_without_metadata > 0 ? "warn" : "ok",
    },
    {
      label: "Items without a poster",
      value: report.items_without_poster,
      tone: report.items_without_poster > 0 ? "warn" : "ok",
    },
    {
      label: "Items without a backdrop",
      value: report.items_without_backdrop,
      tone: report.items_without_backdrop > 0 ? "warn" : "ok",
    },
    {
      label: "Orphan episodes (no file)",
      value: report.orphan_episodes,
      tone: report.orphan_episodes > 0 ? "warn" : "ok",
    },
    {
      label: "Orphan media files (no item/episode)",
      value: report.orphan_media_files,
      tone: report.orphan_media_files > 0 ? "warn" : "ok",
    },
  ];

  return (
    <div className="space-y-6">
      <section className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {counters.map((c) => (
          <div
            key={c.label}
            className="rounded-lg border border-white/10 bg-white/2 p-4"
          >
            <div className="text-xs uppercase tracking-wider text-white/50">
              {c.label}
            </div>
            <div
              className={`mt-2 text-3xl font-bold ${
                c.tone === "ok" ? "text-emerald-300" : "text-amber-300"
              }`}
            >
              {c.value.toLocaleString()}
            </div>
          </div>
        ))}
      </section>

      <section className="rounded-lg border border-white/10 bg-white/2 p-5">
        <h2 className="text-lg font-semibold">
          Missing files on disk ({report.missing_files.length})
        </h2>
        <p className="mt-1 text-xs text-white/50">
          Sample of media file rows whose path no longer exists. The full
          scrub-and-fix workflow belongs to a maintenance task; this is a
          preview limited to the 50 most recent matches.
        </p>
        {report.missing_files.length === 0 ? (
          <p className="mt-3 text-sm text-emerald-300">
            No missing files detected in the sample.
          </p>
        ) : (
          <ul className="mt-3 space-y-1 text-xs">
            {report.missing_files.map((m) => (
              <li
                key={m.id}
                className="flex items-baseline gap-3 border-b border-white/5 py-1.5 last:border-b-0"
              >
                <span className="w-12 shrink-0 font-mono text-white/40">
                  #{m.id}
                </span>
                <span className="min-w-0 flex-1">
                  <div className="truncate font-mono">{m.path}</div>
                  {(m.item_title || m.episode_title) && (
                    <div className="truncate text-white/55">
                      {m.item_title}
                      {m.episode_title ? ` · ${m.episode_title}` : ""}
                    </div>
                  )}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      {report.libraries_without_paths.length > 0 && (
        <section className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-5">
          <h2 className="text-lg font-semibold text-amber-100">
            Libraries with no paths
          </h2>
          <p className="mt-1 text-xs text-amber-100/80">
            These libraries exist but have nothing to scan. Add a path
            under Settings → Libraries.
          </p>
          <ul className="mt-3 space-y-1 text-sm text-amber-100">
            {report.libraries_without_paths.map((l) => (
              <li key={l.id}>
                {l.name} <span className="text-amber-100/60">#{l.id}</span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
