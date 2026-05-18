import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTasksClient } from "@/components/admin/AdminTasksClient";

export default async function AdminTasksPage() {
  const initial = await adminApi.tasks.list();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Scheduled Tasks"
        description="Cron-driven background work. Defaults seeded on first run; edit schedules or add new entries below."
      />
      <AdminTasksClient initial={initial} />
    </div>
  );
}
