import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminOptimizedClient } from "@/components/admin/AdminOptimizedClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminOptimizedPage() {
  const [versions, presets] = await Promise.all([
    adminApi.optimized.list(),
    adminApi.transcoder.listPresets(),
  ]);
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Optimized Versions"
        description={
          <>
            Pre-transcoded copies of source media. Queue a (file × preset)
            pair and the scheduled task{" "}
            <code className="rounded bg-white/10 px-1.5 py-0.5 text-[11px]">
              optimize_versions
            </code>{" "}
            will produce a direct-playable file.
          </>
        }
      />
      <AdminOptimizedClient
        initial={versions.versions}
        presets={presets.presets}
      />
    </div>
  );
}
