import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTasksClient } from "@/components/admin/AdminTasksClient";

export default async function AdminTasksPage() {
  const [initial, settingsRes] = await Promise.all([
    adminApi.tasks.list(),
    adminApi.settings.get(),
  ]);
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Scheduled Tasks"
        description="Toggle background housekeeping. Heavy tasks (scans, backups, deep analysis) run inside the maintenance window so they don't compete with playback. Show Advanced for per-task frequency and custom schedules."
      />
      <AdminTasksClient initial={initial} settings={settingsRes.settings} />
    </div>
  );
}
