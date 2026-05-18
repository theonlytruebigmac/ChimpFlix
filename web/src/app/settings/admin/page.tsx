import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminDashboardClient } from "@/components/admin/AdminDashboardClient";

export default async function AdminDashboardPage() {
  const initial = await adminApi.dashboard();
  return (
    <div>
      <header className="mb-8 flex items-center justify-between gap-4">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Dashboard</h1>
          <p className="mt-1 text-sm text-white/60">
            Live server status — sessions, transcodes, scans, disk usage.
          </p>
        </div>
      </header>
      <AdminDashboardClient initial={initial} />
    </div>
  );
}
