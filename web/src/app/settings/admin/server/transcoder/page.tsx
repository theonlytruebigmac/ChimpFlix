import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminTranscoderClient } from "@/components/admin/AdminTranscoderClient";

export default async function AdminTranscoderPage() {
  const [caps, presets, settings] = await Promise.all([
    adminApi.transcoder.capabilities(),
    adminApi.transcoder.listPresets(),
    adminApi.settings.get(),
  ]);
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Transcoder</h1>
        <p className="mt-1 text-sm text-white/60">
          Hardware acceleration, concurrency, and quality presets.
        </p>
      </header>
      <AdminTranscoderClient
        capabilities={caps.capabilities}
        presets={presets.presets}
        settings={settings.settings}
      />
    </div>
  );
}
