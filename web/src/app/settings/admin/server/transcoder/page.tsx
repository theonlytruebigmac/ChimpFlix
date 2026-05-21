import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTranscoderClient } from "@/components/admin/AdminTranscoderClient";

export default async function AdminTranscoderPage() {
  const [caps, presets, settings] = await Promise.all([
    adminApi.transcoder.capabilities(),
    adminApi.transcoder.listPresets(),
    adminApi.settings.get(),
  ]);
  return (
    <div>
      <AdminPageHeader
        eyebrow="Server"
        title="Transcoder"
        description="Hardware acceleration, concurrency, and quality presets."
      />
      <AdminTranscoderClient
        capabilities={caps.capabilities}
        cacheRoot={caps.cache_root}
        presets={presets.presets}
        settings={settings.settings}
      />
    </div>
  );
}
