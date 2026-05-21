import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTaskFlowClient } from "@/components/admin/AdminTaskFlowClient";

/// Pipeline flow visualization for the media-ingest path. Renders
/// the same data as the overview screen but laid out as a diagram
/// so operators can see the on-add fan-out shape at a glance.
export default async function AdminTasksFlowPage() {
  const overview = await adminApi.tasks.overview();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library · Pipeline flow"
        title="Pipeline flow"
        description="Visual map of the media-ingest pipeline. Click any kind to drill in. Live counts refresh every 5 seconds."
      />
      <AdminTaskFlowClient initial={overview} />
    </div>
  );
}
