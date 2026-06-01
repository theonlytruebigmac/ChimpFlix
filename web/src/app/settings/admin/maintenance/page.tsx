import {
  admin as adminApi,
  libraries as librariesApi,
} from "@/lib/chimpflix-api";
import { AdminMaintenanceTabs } from "@/components/admin/AdminMaintenanceTabs";
import { AdminMaintenanceDashboardClient } from "@/components/admin/AdminMaintenanceDashboardClient";
import { AdminLibraryHealthClient } from "@/components/admin/AdminLibraryHealthClient";
import { AdminBackupRestoreClient } from "@/components/admin/AdminBackupRestoreClient";
import { SettingsBackupClient } from "@/components/SettingsBackupClient";
import { AdminBulkItemsClient } from "@/components/admin/AdminBulkItemsClient";

export const dynamic = "force-dynamic";

const TAB_IDS = ["cleanup", "health", "backups", "bulk"];

/// Maintenance — on-demand cleanup, the library health snapshot, backup &
/// restore, and bulk item operations, consolidated into one tabbed page
/// (folds the old maintenance overview + backup + bulk pages).
export default async function AdminMaintenancePage({
  searchParams,
}: {
  searchParams: Promise<{ tab?: string }>;
}) {
  const sp = await searchParams;
  const initialTab = TAB_IDS.includes(sp?.tab ?? "")
    ? (sp.tab as string)
    : "cleanup";
  const [report, libsRes] = await Promise.all([
    adminApi.libraryHealth(),
    librariesApi.list(),
  ]);
  return (
    <AdminMaintenanceTabs
      initialTab={initialTab}
      cleanup={<AdminMaintenanceDashboardClient />}
      health={<AdminLibraryHealthClient report={report} />}
      backups={
        <div className="space-y-6">
          <section className="rounded-lg border border-white/10 bg-white/2 p-6">
            <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-white/40">
              One-shot download
            </h2>
            <SettingsBackupClient />
          </section>
          <AdminBackupRestoreClient />
        </div>
      }
      bulk={<AdminBulkItemsClient libraries={libsRes.libraries} />}
    />
  );
}
