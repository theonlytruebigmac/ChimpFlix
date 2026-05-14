import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminTasksClient } from "@/components/admin/AdminTasksClient";

export default async function AdminTasksPage() {
  const initial = await adminApi.tasks.list();
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Scheduled Tasks</h1>
        <p className="mt-1 text-sm text-white/60">
          Cron-driven background work. Defaults seeded on first run; edit
          schedules or add new entries below.
        </p>
      </header>
      <AdminTasksClient initial={initial} />
    </div>
  );
}
