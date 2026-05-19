import { AdminBulkItemsClient } from "@/components/admin/AdminBulkItemsClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { libraries as librariesApi } from "@/lib/chimpflix-api";

export default async function AdminBulkItemsPage() {
  const { libraries } = await librariesApi.list();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Bulk operations"
        description={
          <>
            Multi-select items to refresh metadata, add/remove tags, or
            queue marker detection. Useful when a TMDB schema change
            invalidates a batch, or when promoting a tag (&ldquo;rewatch&rdquo;)
            across many titles. Operations cap at 500 items per call.
          </>
        }
      />
      <AdminBulkItemsClient libraries={libraries} />
    </div>
  );
}
