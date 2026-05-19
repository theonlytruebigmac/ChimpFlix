import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminPrerollClient } from "@/components/admin/AdminPrerollClient";
import { preroll as prerollApi, admin as adminApi } from "@/lib/chimpflix-api";

export default async function AdminPrerollPage() {
  const [status, { settings }] = await Promise.all([
    prerollApi.status(),
    adminApi.settings.get(),
  ]);
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Pre-roll"
        description={
          <>
            Upload a short video that plays before every watch session
            starts. Useful for content warnings, sponsor stings, or that
            obligatory THX-style theatrical sting. Plex calls this
            &ldquo;Pre-Rolls&rdquo;; we cap at one file.
          </>
        }
      />
      <AdminPrerollClient
        initialStatus={status}
        initialEnabled={settings.preroll_enabled}
        initialVolume={settings.preroll_volume ?? 100}
      />
    </div>
  );
}
