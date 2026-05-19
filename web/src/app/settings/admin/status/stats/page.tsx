import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminStatsClient } from "@/components/admin/AdminStatsClient";
import { requireAdmin } from "@/lib/chimpflix-server";

// Force-dynamic so the rendered HTML reflects the latest events even
// across full page refreshes — the client polls now-playing on top of
// the SSR-loaded snapshot for the interactive view.
export const dynamic = "force-dynamic";

export default async function AdminStatsPage() {
  await requireAdmin("/settings/admin/status/stats");
  // SSR the initial 30-day snapshot in parallel so first paint shows
  // the four hero numbers + the first 50 activity rows + both charts
  // without the client having to round-trip on hydrate.
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
    <div>
      <AdminPageHeader
        eyebrow="Status"
        title="Stats"
        description="Playback dashboard — recent activity, top users, top titles, transcode mix, platform breakdown, and live sessions. Recorded automatically from every stream start and the player's auto-scrobble event; nothing to enable."
      />
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
    </div>
  );
}
