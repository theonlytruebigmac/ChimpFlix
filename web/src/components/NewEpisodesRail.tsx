import { NewEpisodeCard } from "@/components/NewEpisodeCard";
import { items as itemsApi } from "@/lib/chimpflix-api";

/// "New Episodes" home rail — shows that just gained a fresh episode of a
/// series the user already had. The complement to "Recently Added", which is
/// sorted by each title's first-acquisition `added_at` and so structurally
/// can't resurface an ongoing show when a new episode lands (the show's
/// `added_at` is MIN-converged at its earliest episode). This rail keys off
/// each episode's own `added_at` instead.
///
/// Renders nothing when no show has a recent new episode, so it disappears
/// cleanly on libraries with no fresh episodes — same silent-no-op contract
/// as the other optional rails.
export async function NewEpisodesRail({
  visibleLibIds,
}: {
  visibleLibIds: number[];
}) {
  if (visibleLibIds.length === 0) return null;
  let episodes;
  try {
    const res = await itemsApi.recentlyAddedEpisodes({
      days: 30,
      limit: 24,
      library_ids: visibleLibIds,
    });
    episodes = res.episodes;
  } catch {
    // Feed hiccup — silent no-op so it never blocks the home page.
    return null;
  }
  if (episodes.length === 0) return null;
  // Snapshot one reference instant for every tile's "added X ago" label, so
  // the row is internally consistent and matches the SSR/CSR purity rule.
  // Server component renders once per request — the snapshot is intentional.
  // eslint-disable-next-line react-hooks/purity
  const nowMs = Date.now();

  return (
    <section
      className="zf-rise-in px-4 pb-1 pt-1 sm:px-8 md:px-12"
      style={{ contentVisibility: "auto", containIntrinsicSize: "260px" }}
    >
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h2 className="text-lg font-semibold tracking-tight sm:text-xl md:text-[1.4rem]">
          New Episodes
        </h2>
      </div>
      <div className="flex gap-3 overflow-x-auto overscroll-x-contain touch-pan-x pb-2 sm:gap-4">
        {episodes.map((ep) => (
          <NewEpisodeCard
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
