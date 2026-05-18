import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminMaintenanceDashboardClient } from "@/components/admin/AdminMaintenanceDashboardClient";

export default async function AdminMaintenancePage() {
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Overview"
        description={
          <>
            One-click cleanup across the entire instance — verify all
            libraries against disk, purge orphaned rows, vacuum the
            database, and clear stale transcoder cache. Each action
            mirrors a scheduled task (which runs on its own cadence);
            these buttons are the on-demand path.
          </>
        }
      />
      <AdminMaintenanceDashboardClient />
    </div>
  );
}
