import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminDashboardClient } from "@/components/admin/AdminDashboardClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminDashboardPage() {
  const initial = await adminApi.dashboard();
  return (
    <div>
      <AdminPageHeader
        title="Home"
        description="System health, recent activity, and alerts at a glance. Drill in for details."
      />
      <AdminDashboardClient initial={initial} />
    </div>
  );
}
