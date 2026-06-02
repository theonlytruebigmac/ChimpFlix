import { CalendarClient } from "@/components/CalendarClient";
import { ModalRoot } from "@/components/ModalRoot";
import {
  items as itemsApi,
  libraries as librariesApi,
  prefs as prefsApi,
} from "@/lib/chimpflix-api";
import { requireUser } from "@/lib/chimpflix-server";

/// Calendar — the LOCAL-data complement to the Trakt-driven coming-soon
/// rails. Lists every locally-known episode whose air date falls in a window
/// (a short look-back so "this week" still shows, plus ~5 weeks ahead),
/// grouped by calendar day. Honors the same per-library visibility +
/// kids-safe rules as every other browse surface (the feed filters
/// server-side; we additionally pass the user's visible-library set so
/// hidden libraries never leak in).
///
/// The day-grouping + relative-day headings ("Today" / "Tomorrow" /
/// "Saturday, Jun 6") happen on the client (`CalendarClient`) because they
/// depend on the browser's local timezone.
export default async function CalendarPage() {
  await requireUser("/calendar");
  const [{ libraries: allLibs }, { library_ids: hiddenIds }] =
    await Promise.all([librariesApi.list(), prefsApi.hiddenLibraries()]);
  const hidden = new Set(hiddenIds);
  const visibleLibIds = allLibs
    .filter((l) => l.visibility !== "hidden" && !hidden.has(l.id))
    .map((l) => l.id);

  // No visible libraries → no feed to fetch (the backend treats an empty
  // library_ids as "no filter", so we must short-circuit rather than send
  // an empty list, same as the home page's empty-state guard).
  let episodes: Awaited<ReturnType<typeof itemsApi.calendar>>["episodes"] = [];
  if (visibleLibIds.length > 0) {
    try {
      const res = await itemsApi.calendar({
        days: 35,
        limit: 300,
        library_ids: visibleLibIds,
      });
      episodes = res.episodes;
    } catch {
      // Feed hiccup — render the empty state rather than a hard error.
      episodes = [];
    }
  }

  return (
    <main className="relative min-h-screen bg-background">
      <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
        <div className="mb-8">
          <div className="text-[0.7rem] font-semibold uppercase tracking-[0.18em] text-accent">
            Coming Up
          </div>
          <h1 className="mt-1 text-4xl font-bold tracking-tight">Calendar</h1>
          <p className="mt-1 text-sm text-white/55">
            Upcoming episodes from your libraries, by air date.
          </p>
        </div>
        <CalendarClient episodes={episodes} />
      </div>
      <ModalRoot />
    </main>
  );
}
