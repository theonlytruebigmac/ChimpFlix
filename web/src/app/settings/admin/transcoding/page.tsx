import {
  admin as adminApi,
  preroll as prerollApi,
} from "@/lib/chimpflix-api";
import { AdminTranscodingTabs } from "@/components/admin/AdminTranscodingTabs";

/// Transcoding — hardware acceleration, concurrency, quality, and the
/// pre-roll bumper. Folds the old transcoder + preroll pages into one tabbed
/// surface; both tabs' data is fetched up front so switching is instant.
export default async function AdminTranscodingPage({
  searchParams,
}: {
  searchParams: Promise<{ tab?: string }>;
}) {
  const sp = await searchParams;
  const initialTab = sp?.tab === "preroll" ? "preroll" : "engine";
  const [caps, presets, { settings }, prerollStatus] = await Promise.all([
    adminApi.transcoder.capabilities(),
    adminApi.transcoder.listPresets(),
    adminApi.settings.get(),
    prerollApi.status(),
  ]);
  return (
    <AdminTranscodingTabs
      initialTab={initialTab}
      engine={{
        capabilities: caps.capabilities,
        cacheRoot: caps.cache_root,
        settings,
      }}
      presets={{
        presets: presets.presets,
      }}
      preroll={{
        initialStatus: prerollStatus,
        initialEnabled: settings.preroll_enabled,
        initialVolume: settings.preroll_volume ?? 100,
      }}
    />
  );
}
