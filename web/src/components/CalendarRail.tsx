import Link from "next/link";
import { CalendarEpisodeCard } from "@/components/CalendarEpisodeCard";
import { items as itemsApi } from "@/lib/chimpflix-api";

/// "Coming Up" home rail — the LOCAL-data complement to the Trakt-driven
/// `ComingSoonRail`. Driven entirely by the `/calendar` feed (episodes whose
/// `air_date` falls in a window, honoring the user's per-library visibility +
/// kids-safe rules server-side), so it works with no Trakt link at all.
///
/// Renders nothing when nothing is upcoming, so it disappears cleanly on
/// libraries with no scheduled episodes — same silent-no-op contract as the
/// other optional rails.
export async function CalendarRail({
  visibleLibIds,
}: {
  visibleLibIds: number[];
}) {
  if (visibleLibIds.length === 0) return null;
  let episodes;
  try {
    const res = await itemsApi.calendar({
      days: 35,
      limit: 24,
      library_ids: visibleLibIds,
    });
    episodes = res.episodes;
  } catch {
    // Calendar feed hiccup — silent no-op so it never blocks the home page.
    return null;
  }
  if (episodes.length === 0) return null;
  // Snapshot one reference instant for every tile's relative-day label, so
  // the row is internally consistent and matches the SSR/CSR purity rule.
  const nowMs = Date.now();

  return (
    <section
      className="zf-rise-in px-4 pb-1 pt-1 sm:px-8 md:px-12"
      style={{ contentVisibility: "auto", containIntrinsicSize: "260px" }}
    >
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h2 className="text-lg font-semibold tracking-tight sm:text-xl md:text-[1.4rem]">
          Coming Up
        </h2>
        <Link
          href="/calendar"
          className="shrink-0 text-sm font-medium text-white/60 transition-colors hover:text-accent focus:outline-none focus-visible:text-accent"
        >
          see all &rarr;
        </Link>
      </div>
      <div className="flex gap-3 overflow-x-auto overscroll-x-contain touch-pan-x pb-2 sm:gap-4">
        {episodes.map((ep) => (
          <CalendarEpisodeCard
            key={ep.episodeId}
            episode={ep}
            nowMs={nowMs}
            className="w-72 shrink-0 sm:w-80"
          />
        ))}
      </div>
    </section>
  );
}
