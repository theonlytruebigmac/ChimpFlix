import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminLibrarySettingsClient } from "@/components/admin/AdminLibrarySettingsClient";

export default async function AdminLibrarySettingsPage() {
  const settingsRes = await adminApi.settings.get();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Library Settings"
        description="Server-wide library defaults — auto-scan, Continue Watching dials, database tuning. Per-library knobs (paths, visibility, allow-deletion) live in Library › Libraries."
      />
      <AdminLibrarySettingsClient settings={settingsRes.settings} />
    </div>
  );
}
