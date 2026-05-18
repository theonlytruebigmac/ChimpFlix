import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminOptimizedClient } from "@/components/admin/AdminOptimizedClient";

export default async function AdminOptimizedPage() {
  const [versions, presets] = await Promise.all([
    adminApi.optimized.list(),
    adminApi.transcoder.listPresets(),
  ]);
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">
          Optimized Versions
        </h1>
        <p className="mt-1 text-sm text-white/60">
          Pre-transcoded copies of source media. Queue a (file × preset)
          pair and the scheduled task <code className="rounded bg-white/10 px-1">optimize_versions</code>{" "}
          will produce a direct-playable file.
        </p>
      </header>
      <AdminOptimizedClient
        initial={versions.versions}
        presets={presets.presets}
      />
    </div>
  );
}
