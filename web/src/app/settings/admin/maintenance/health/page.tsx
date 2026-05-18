import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLibraryHealthClient } from "@/components/admin/AdminLibraryHealthClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export const dynamic = "force-dynamic";

export default async function AdminLibraryHealthPage() {
  const report = await adminApi.libraryHealth();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Library health"
        description="A read-only snapshot of common library pathologies: orphaned rows, missing files on disk, items lacking artwork or metadata ids. Refresh the page to re-run the checks."
      />
      <AdminLibraryHealthClient report={report} />
    </div>
  );
}
