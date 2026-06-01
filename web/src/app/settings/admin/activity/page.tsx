import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminStatsClient } from "@/components/admin/AdminStatsClient";
import { requireAdmin } from "@/lib/chimpflix-server";

// Force-dynamic so the rendered HTML reflects the latest events even
// across full page refreshes — the client polls now-playing on top of
// the SSR-loaded snapshot for the interactive view.
export const dynamic = "force-dynamic";

/// Server activity & stats — playback dashboard (recent activity, top
/// users/titles/platforms/libraries, transcode mix, live sessions, and the
/// per-day/per-hour histograms). Recorded automatically from every stream
/// start; nothing to enable. (Was /settings/admin/status/stats.)
export default async function AdminActivityPage() {
  await requireAdmin("/settings/admin/activity");
  // SSR the initial 30-day snapshot in parallel so first paint shows the
  // hero numbers + activity rows + charts without a hydrate round-trip.
  const [
    overview,
    activity,
    topUsers,
    topItems,
    topPlatforms,
    topLibraries,
    nowPlaying,
    perDay,
    perHour,
  ] = await Promise.all([
    adminApi.stats.overview(30),
    adminApi.stats.activity({ limit: 50 }),
    adminApi.stats.topUsers({ days: 30, limit: 10 }),
    adminApi.stats.topItems({ days: 30, limit: 10 }),
    adminApi.stats.topPlatforms({ days: 30, limit: 8 }),
    adminApi.stats.topLibraries({ days: 30, limit: 10 }),
    adminApi.stats.nowPlaying(),
    adminApi.stats.playsPerDay(30),
    adminApi.stats.playsPerHour(30),
  ]);
  return (
    <AdminStatsClient
      initialOverview={overview}
      initialActivity={activity.events}
      initialTopUsers={topUsers.users}
      initialTopItems={topItems.items}
      initialTopPlatforms={topPlatforms.platforms}
      initialTopLibraries={topLibraries.libraries}
      initialNowPlaying={nowPlaying.sessions}
      initialPerDay={perDay.buckets}
      initialPerHour={perHour.buckets}
    />
  );
}
