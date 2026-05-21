import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLibraryHealthClient } from "@/components/admin/AdminLibraryHealthClient";
import { AdminMaintenanceDashboardClient } from "@/components/admin/AdminMaintenanceDashboardClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export const dynamic = "force-dynamic";

/// Maintenance overview — operator-facing health and on-demand
/// cleanup tools in one surface. Used to be split across two
/// sidebar entries (Overview + Library Health); folded together
/// because they answer the same question ("how is the system
/// holding up, and what can I run to fix it?") and the library
/// health snapshot reads cheaply enough to render inline.
export default async function AdminMaintenancePage() {
  const report = await adminApi.libraryHealth();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Overview"
        description={
          <>
            One-click cleanup across the entire instance, plus a
            read-only health snapshot of the libraries themselves.
            Cleanup buttons mirror scheduled tasks (which run on a
            cadence); these are the on-demand path.
          </>
        }
      />
      <AdminMaintenanceDashboardClient />
      <section className="mt-10">
        <h2 className="mb-4 text-base font-semibold tracking-tight text-white/90">
          Library health
        </h2>
        <p className="-mt-3 mb-5 max-w-2xl text-sm text-white/55">
          Common library pathologies: orphaned rows, missing files on
          disk, items lacking artwork or metadata ids. Reload the page
          to re-run the checks.
        </p>
        <AdminLibraryHealthClient report={report} />
      </section>
    </div>
  );
}
